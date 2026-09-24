// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Packages, and the catalogue that holds them.
//!
//! A [`Package`] is a manifest compiled for use: its templates parsed, its
//! `let` bindings checked, its fixtures loaded. A [`Catalogue`] is every
//! package the app knows, findable by id or alias, and it is what turns a
//! clip's applied effects into one FFmpeg chain.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use concat_project::model::AppliedFilter;
use serde::Deserialize;

use crate::Error;
use crate::expr::{Expr, Value};
use concat_core::{Lut, RevealMap, ShaderPass, TransitionPass};

use crate::manifest::{Kind, Manifest};
use crate::shader::{Shader, TransitionShader};
use crate::template::Template;

/// The author every built-in package is under, and no other package may
/// be: the front of an id, before the dot.
pub const BUILTIN_AUTHOR: &str = "concat";

/// A compiled FFmpeg backend.
#[derive(Clone, Debug)]
struct Chain {
    lets: Vec<(String, Expr)>,
    template: Template,
}

/// The key every filter answers to without declaring it: how much of the
/// look is applied, as a percent. Absent means all of it.
pub const INTENSITY: &str = "intensity";

/// A filter's fragment, mixed back with the untouched picture by its
/// intensity. At a hundred the fragment is returned as it was; below it the
/// picture is split, the look runs on one copy, and the two are blended by
/// the fraction - which is what one intensity slider means on every look
/// there is, and why no package has to implement it. The labels carry the
/// fragment's index so two mixed links in one chain never share a name.
fn mixed(
    package: &Package,
    params: &BTreeMap<String, f64>,
    fragment: String,
    index: usize,
) -> String {
    if package.kind() != Kind::Filter {
        return fragment;
    }
    let mix = params
        .get(INTENSITY)
        .copied()
        .unwrap_or(100.0)
        .clamp(0.0, 100.0)
        / 100.0;
    if mix >= 1.0 {
        return fragment;
    }
    format!(
        "split[m{index}a][m{index}b];[m{index}b]{fragment}[m{index}c];\
         [m{index}a][m{index}c]blend=all_mode=normal:all_opacity={mix:.3}"
    )
}

/// One effect, ready to use.
#[derive(Clone, Debug)]
pub struct Package {
    /// What the package declares.
    pub manifest: Manifest,
    chain: Option<Chain>,
    /// The shader, when the package has one: the GPU renders it, and the
    /// chain - if the package has that too - is the CPU's fallback.
    shader: Option<Shader>,
    /// The two-input shader, when the package is a transition: the GPU
    /// combines the outgoing and incoming pictures with it.
    transition: Option<TransitionShader>,
    /// The pinned outputs shipped with the package.
    pub fixtures: Vec<Fixture>,
    /// The folder the package was loaded from; None for a built-in.
    pub folder: Option<std::path::PathBuf>,
    /// The table the manifest's `[lut]` names, read at load.
    lut: Option<Arc<Lut>>,
}

/// Where a fixture's parameters start from.
#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum At {
    /// Every parameter at its default.
    #[default]
    Default,
    /// Every parameter at its minimum.
    Min,
    /// Every parameter at its maximum.
    Max,
}

/// One pinned case from `fixtures.toml`: these parameters produce exactly
/// this chain.
#[derive(Deserialize, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    /// A name for the failure message.
    #[serde(default)]
    pub name: String,
    /// The starting point every parameter takes.
    #[serde(default)]
    pub at: At,
    /// Parameters set on top of `at`.
    #[serde(default)]
    pub params: BTreeMap<String, f64>,
    /// The emitted position the fragment is rendered at.
    #[serde(default)]
    pub index: usize,
    /// The expected chain.
    pub chain: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureFile {
    #[serde(default, rename = "case")]
    cases: Vec<Fixture>,
}

impl Package {
    /// Compiles a package from the text of its manifest and, if it ships
    /// them, its fixtures file and its shader.
    pub fn from_sources(
        manifest: &str,
        fixtures: Option<&str>,
        shader: Option<&str>,
    ) -> Result<Package, Error> {
        Package::from_manifest(Manifest::parse(manifest)?, fixtures, shader, None)
    }

    /// `from_sources` with the manifest already parsed, and the table its
    /// `[lut]` names read from `folder`, when it has one.
    fn from_manifest(
        manifest: Manifest,
        fixtures: Option<&str>,
        shader: Option<&str>,
        table: Option<(std::path::PathBuf, Arc<Lut>)>,
    ) -> Result<Package, Error> {
        let invalid = |message: String| Error::Invalid {
            id: manifest.effect.id.clone(),
            message,
        };

        // The shader file beside the manifest, if any: a `[wgsl]` effect's
        // body, or a `[transition]`'s two-input body.
        let body = shader;
        // A `[wgsl]` table without the file, or the file without the table,
        // is a package that does not know what it is.
        let shader = match (&manifest.wgsl, body) {
            (Some(_), Some(body)) => Some(
                Shader::compile(&manifest, body)
                    .map_err(|message| invalid(format!("shader: {message}")))?,
            ),
            (Some(wgsl), None) => {
                return Err(invalid(format!(
                    "[wgsl] names `{}` but no shader was given",
                    wgsl.entry
                )));
            }
            // A transition's `effect.wgsl` is its two-input shader, compiled
            // just below, not a stray effect shader.
            (None, Some(_)) if manifest.transition.is_some() => None,
            (None, Some(_)) => {
                return Err(invalid(
                    "a shader was given but the manifest has no [wgsl] table".to_owned(),
                ));
            }
            (None, None) => None,
        };
        let transition = match (&manifest.transition, body) {
            (Some(_), Some(body)) => Some(
                TransitionShader::compile(&manifest, body)
                    .map_err(|message| invalid(format!("transition shader: {message}")))?,
            ),
            (Some(table), None) => {
                return Err(invalid(format!(
                    "[transition] names `{}` but no shader was given",
                    table.entry
                )));
            }
            (None, _) => None,
        };
        if shader.is_none()
            && transition.is_none()
            && manifest.ffmpeg.is_none()
            && manifest.effect.kind != Kind::Audio
        {
            return Err(invalid(
                "the package has neither a shader nor a chain".to_owned(),
            ));
        }

        let chain = match &manifest.ffmpeg {
            None => None,
            Some(ffmpeg) => {
                let mut known: Vec<String> =
                    manifest.params.iter().map(|p| p.key.clone()).collect();
                known.push("index".to_owned());
                if manifest.lut.is_some() {
                    known.push("lut".to_owned());
                }
                let mut lets = Vec::new();
                for binding in &ffmpeg.lets {
                    let Some((name, source)) = binding.split_once('=') else {
                        return Err(invalid(format!(
                            "let `{binding}` is not `name = expression`"
                        )));
                    };
                    let name = name.trim();
                    if known.iter().any(|k| k == name) {
                        return Err(invalid(format!(
                            "let `{name}` shadows a name already defined"
                        )));
                    }
                    let expr = Expr::parse(source.trim())
                        .map_err(|error| invalid(format!("let `{name}`: {error}")))?;
                    check_names(&expr, &known).map_err(invalid)?;
                    known.push(name.to_owned());
                    lets.push((name.to_owned(), expr));
                }
                // A filter that reads a file reads it with the host's
                // rights: the only file a chain may name is the package's
                // own look-up table, and only through `{lut}`.
                if let Some(option) = names_a_file(&ffmpeg.chain) {
                    return Err(invalid(format!(
                        "chain names a file through `{option}`: a package's chain may read \
                         its own look-up table as `{{lut}}` and no other file"
                    )));
                }
                let template = Template::parse(&ffmpeg.chain)
                    .map_err(|error| invalid(format!("chain: {error}")))?;
                let mut names = Vec::new();
                template.names(&mut names);
                for name in names {
                    if !known.contains(&name) {
                        return Err(invalid(format!(
                            "chain reads `{name}`, which is not declared"
                        )));
                    }
                }
                // A package is a folder anyone can share; its chain may
                // touch the frame and nothing else. See `filters`.
                for filter in template
                    .filters()
                    .map_err(|error| invalid(format!("chain: {error}")))?
                {
                    if !crate::filters::allowed(&filter) {
                        return Err(invalid(format!(
                            "chain uses `{filter}`, which is not a filter a package may use: \
                             a package's chain reads the frame and writes the frame, and \
                             nothing else"
                        )));
                    }
                }
                Some(Chain { lets, template })
            }
        };

        let fixtures = match fixtures {
            None => Vec::new(),
            Some(source) => {
                let file: FixtureFile = toml::from_str(source)
                    .map_err(|error| invalid(format!("fixtures: {error}")))?;
                file.cases
            }
        };

        if manifest.lut.is_some() && table.is_none() {
            // The table is a file beside the manifest, and only a folder
            // has one; see `from_folder`.
            return Err(invalid(
                "a package with a [lut] must be loaded from its folder".to_owned(),
            ));
        }
        let (folder, lut) = match table {
            Some((folder, lut)) => (Some(folder), Some(lut)),
            None => (None, None),
        };
        let package = Package {
            manifest,
            chain,
            shader,
            transition,
            fixtures,
            folder,
            lut,
        };
        // Render at every bound now, so a type error in an expression is a
        // load failure and never a silent gap in an export.
        if package.chain.is_some() {
            for at in [At::Default, At::Min, At::Max] {
                package
                    .ffmpeg_fragment(&package.params_at(at), 0)
                    .map_err(|error| Error::Invalid {
                        id: package.id().to_owned(),
                        message: format!("chain at {at:?}: {error}"),
                    })?;
            }
        }
        Ok(package)
    }

    /// The namespaced id.
    pub fn id(&self) -> &str {
        &self.manifest.effect.id
    }

    /// The category declared in the manifest.
    pub fn category(&self) -> &str {
        &self.manifest.effect.category
    }

    /// Whether this package belongs to or matches a category filter.
    pub fn matches_category(&self, category: &str) -> bool {
        let cat = self.category();
        if category.is_empty() || category.eq_ignore_ascii_case("All") {
            return true;
        }
        if cat.eq_ignore_ascii_case(category) {
            return true;
        }
        match category {
            "Featured" => {
                cat.eq_ignore_ascii_case("Featured")
                    || self.manifest.effect.order < 10
                    || matches!(
                        self.id(),
                        "concat.camera-shake"
                            | "concat.rgb-glitch"
                            | "concat.sparkle"
                            | "concat.light-leak"
                            | "concat.tilt-shift"
                            | "concat.strobe-flash"
                            | "concat.glow"
                            | "concat.bloom-pulse"
                    )
            }
            "Retro & Film" => {
                cat.eq_ignore_ascii_case("Retro")
                    || cat.eq_ignore_ascii_case("Film")
                    || cat.eq_ignore_ascii_case("Cinematic")
                    || cat.eq_ignore_ascii_case("Retro & Film")
                    || matches!(
                        self.id(),
                        "concat.crt-scanlines"
                            | "concat.film-reel"
                            | "concat.camcorder-90s"
                            | "concat.halation"
                            | "concat.vhs"
                            | "concat.film-grain"
                            | "concat.scanlines"
                            | "concat.dust"
                            | "concat.falling-dust"
                    )
            }
            "Optical & Lens" => {
                cat.eq_ignore_ascii_case("Optical")
                    || cat.eq_ignore_ascii_case("Lens")
                    || cat.eq_ignore_ascii_case("Blur")
                    || cat.eq_ignore_ascii_case("Optical & Lens")
                    || matches!(
                        self.id(),
                        "concat.tilt-shift"
                            | "concat.prism-dispersion"
                            | "concat.fisheye"
                            | "concat.lens-flare"
                            | "concat.bokeh"
                            | "concat.gaussian-blur"
                            | "concat.box-blur"
                            | "concat.motion-blur"
                    )
            }
            "Distortion & Glitch" => {
                cat.eq_ignore_ascii_case("Distort")
                    || cat.eq_ignore_ascii_case("Glitch")
                    || cat.eq_ignore_ascii_case("Distortion & Glitch")
                    || matches!(
                        self.id(),
                        "concat.wave-warp"
                            | "concat.vortex-swirl"
                            | "concat.prism-dispersion"
                            | "concat.fisheye"
                            | "concat.mirror-tile"
                            | "concat.rgb-glitch"
                            | "concat.datamosh"
                            | "concat.ripple"
                            | "concat.twirl"
                            | "concat.bulge-pinch"
                            | "concat.swirl"
                            | "concat.mirror"
                    )
            }
            "Party & Club" => {
                cat.eq_ignore_ascii_case("Party")
                    || cat.eq_ignore_ascii_case("Club")
                    || cat.eq_ignore_ascii_case("Party & Club")
                    || matches!(
                        self.id(),
                        "concat.bass-shockwave"
                            | "concat.laser-beams"
                            | "concat.neon-glow"
                            | "concat.color-cycle"
                            | "concat.strobe-flash"
                            | "concat.neon"
                            | "concat.neon-edges"
                    )
            }
            "Light & Shadow" => {
                cat.eq_ignore_ascii_case("Light")
                    || cat.eq_ignore_ascii_case("Shadow")
                    || cat.eq_ignore_ascii_case("Light & Shadow")
                    || matches!(
                        self.id(),
                        "concat.sparkle"
                            | "concat.light-leak"
                            | "concat.strobe-flash"
                            | "concat.halation"
                            | "concat.glow"
                            | "concat.bloom-pulse"
                            | "concat.lens-flare"
                    )
            }
            _ => false,
        }
    }

    /// Which catalogue the package belongs to.
    pub fn kind(&self) -> Kind {
        self.manifest.effect.kind
    }

    /// The pass this package's shader makes at its defaults: what a vet
    /// runs once before the package is offered. None for a package with
    /// no shader.
    pub fn trial_pass(&self) -> Option<ShaderPass> {
        let shader = self.shader.as_ref()?;
        let values = self.resolve(&BTreeMap::new());
        Some(shader.pass(&values, &self.manifest.params, 1.0, self.lut.clone(), None))
    }

    /// The shader, when the package renders on the GPU.
    pub fn shader(&self) -> Option<&Shader> {
        self.shader.as_ref()
    }

    /// The two-input shader, when the package is a transition.
    pub fn transition(&self) -> Option<&TransitionShader> {
        self.transition.as_ref()
    }

    /// The legacy transition id this package degrades to where its shader
    /// cannot run - the CPU reference and export without a GPU. `cross-fade`
    /// unless the manifest names another.
    pub fn transition_fallback(&self) -> &str {
        self.manifest
            .transition
            .as_ref()
            .and_then(|table| table.fallback.as_deref())
            .unwrap_or("cross-fade")
    }

    /// The FFmpeg `xfade` name this transition declares as its shape for a
    /// compositor that runs no shaders - the CPU reference, and an export
    /// or monitor without a GPU - if it declares one. The name is FFmpeg's
    /// so a manifest can be checked against a known list at load, but what
    /// draws it is `concat_render`, in the shipped shader's own terms.
    pub fn transition_xfade(&self) -> Option<&str> {
        self.manifest
            .transition
            .as_ref()
            .and_then(|table| table.xfade.as_deref())
    }

    /// Whether `id` is this package's id or one of its aliases.
    pub fn answers_to(&self, id: &str) -> bool {
        self.id() == id || self.manifest.effect.aliases.iter().any(|alias| alias == id)
    }

    /// Every declared parameter at `at`.
    pub fn params_at(&self, at: At) -> BTreeMap<String, f64> {
        self.manifest
            .params
            .iter()
            .map(|param| {
                let value = match at {
                    At::Default => param.default,
                    At::Min => param.min,
                    At::Max => param.max,
                };
                (param.key.clone(), value)
            })
            .collect()
    }

    /// Every declared parameter: the value in `set`, or the default. Keys
    /// the manifest does not declare are dropped.
    pub fn resolve(&self, set: &BTreeMap<String, f64>) -> BTreeMap<String, f64> {
        self.manifest
            .params
            .iter()
            .map(|param| {
                let value = set.get(&param.key).copied().unwrap_or(param.default);
                (param.key.clone(), value)
            })
            .collect()
    }

    /// The FFmpeg fragment for these parameters, or `None` when the package
    /// has no FFmpeg backend. `index` is the fragment's emitted position in
    /// the clip's chain; any filtergraph labels must embed it.
    pub fn ffmpeg_fragment(
        &self,
        set: &BTreeMap<String, f64>,
        index: usize,
    ) -> Result<Option<String>, Error> {
        let Some(chain) = &self.chain else {
            return Ok(None);
        };
        let mut env: BTreeMap<String, Value> = self
            .resolve(set)
            .into_iter()
            .map(|(key, value)| (key, Value::Float(value)))
            .collect();
        env.insert("index".to_owned(), Value::Int(index as i64));
        if let (Some(table), Some(folder)) = (&self.manifest.lut, &self.folder) {
            let path = folder.join(&table.file);
            env.insert(
                "lut".to_owned(),
                Value::Text(escape_option(&path.to_string_lossy())),
            );
        }
        let invalid = |message: String| Error::Invalid {
            id: self.id().to_owned(),
            message,
        };
        for (name, expr) in &chain.lets {
            let value = expr
                .eval(&env)
                .map_err(|error| invalid(format!("let `{name}`: {error}")))?;
            env.insert(name.clone(), value);
        }
        chain
            .template
            .render(&env)
            .map(Some)
            .map_err(|error| invalid(format!("chain: {error}")))
    }

    /// Loads a package from its folder: the manifest, and beside it the
    /// fixtures, the shader and the table, whichever the manifest names.
    pub fn from_folder(folder: &Path) -> Result<Package, Error> {
        let read = |name: &str| {
            std::fs::read_to_string(folder.join(name)).map_err(|error| Error::Io {
                path: folder.join(name),
                message: error.to_string(),
            })
        };
        let manifest_text = read("effect.toml")?;
        let fixtures = read("fixtures.toml").ok();
        let shader = read("effect.wgsl").ok();
        let manifest = Manifest::parse(&manifest_text)?;
        // The built-ins' author is theirs alone. A package of the user's
        // own under it would shadow, or be shadowed by, one that ships
        // with the app, and which won would turn on load order.
        let reserved = std::iter::once(manifest.effect.id.as_str())
            .chain(manifest.effect.aliases.iter().map(String::as_str))
            .find(|name| {
                name.split_once('.')
                    .is_some_and(|(author, _)| author == BUILTIN_AUTHOR)
            });
        if let Some(name) = reserved {
            return Err(Error::Invalid {
                id: manifest.effect.id.clone(),
                message: format!(
                    "`{name}` is under `{BUILTIN_AUTHOR}.`, the author of the packages that \
                     ship with the app; a package of your own is `author.name`"
                ),
            });
        }
        let table = match &manifest.lut {
            Some(table) => {
                let text = read(&table.file)?;
                let lut = crate::cube::parse(&text).map_err(|message| Error::Invalid {
                    id: manifest.effect.id.clone(),
                    message: format!("{}: {message}", table.file),
                })?;
                Some(Arc::new(lut))
            }
            None => None,
        };
        let mut package = Package::from_manifest(
            manifest,
            fixtures.as_deref(),
            shader.as_deref(),
            table.map(|lut| (folder.to_path_buf(), lut)),
        )?;
        if package.folder.is_none() {
            package.folder = Some(folder.to_path_buf());
        }
        Ok(package)
    }

    /// The table the package ships, if it does.
    pub fn lut(&self) -> Option<&Arc<Lut>> {
        self.lut.as_ref()
    }

    /// Renders every fixture and reports the ones whose chain came out
    /// different: the package's own regression suite.
    pub fn check_fixtures(&self) -> Vec<String> {
        let mut failures = Vec::new();
        for (n, case) in self.fixtures.iter().enumerate() {
            let mut params = self.params_at(case.at);
            for (key, value) in &case.params {
                params.insert(key.clone(), *value);
            }
            let label = if case.name.is_empty() {
                format!("case {}", n + 1)
            } else {
                case.name.clone()
            };
            match self.ffmpeg_fragment(&params, case.index) {
                Ok(Some(chain)) if chain == case.chain => {}
                Ok(Some(chain)) => failures.push(format!(
                    "{}: {label}\n  expected {}\n  rendered {chain}",
                    self.id(),
                    case.chain
                )),
                Ok(None) => failures.push(format!("{}: {label}: no ffmpeg backend", self.id())),
                Err(error) => failures.push(format!("{}: {label}: {error}", self.id())),
            }
        }
        failures
    }

    /// Everything wrong with the package in `folder`, one line each, or
    /// nothing: the check an author runs before sharing a package, and
    /// what `concat-cli check` prints. The folder is loaded the way the
    /// window loads it, so a manifest, template or shader fault comes out
    /// here rather than in someone else's log; its id and aliases are held
    /// against `taken`, the catalogue it would join; and its fixtures run.
    pub fn check_folder(folder: &Path, taken: &Catalogue) -> Vec<String> {
        let package = match Package::from_folder(folder) {
            Ok(package) => package,
            Err(error) => return vec![error.to_string()],
        };
        let mut problems = Vec::new();
        let mut names = vec![package.id().to_owned()];
        names.extend(package.manifest.effect.aliases.iter().cloned());
        for name in names {
            if let Some(other) = taken.get(&name) {
                problems.push(format!(
                    "{}: `{name}` is already taken by `{}`",
                    package.id(),
                    other.id()
                ));
            }
        }
        problems.extend(package.check_fixtures());
        problems
    }
}

/// The option a chain names a file through, if it does: `file=`,
/// `psfile=` and `filename=` are how FFmpeg's filters take one, and the
/// only value a package may give is its own table, `{lut}`.
fn names_a_file(chain: &str) -> Option<&'static str> {
    const OPTIONS: [&str; 3] = ["file=", "psfile=", "filename="];
    for option in OPTIONS {
        let mut rest = chain;
        while let Some(at) = rest.find(option) {
            let before = rest[..at].chars().next_back();
            let value = &rest[at + option.len()..];
            let named = before.is_none_or(|c| !c.is_alphanumeric() && c != '_');
            if named && !value.starts_with("{lut}") {
                return Some(option);
            }
            rest = &rest[at + option.len()..];
        }
    }
    None
}

/// A number that changes when anything in the package folders under `dir`
/// does: a folder added or taken away, a file in one written, added or
/// removed. What a watcher polls, cheaply - a stat per file, no reading -
/// to know when the folder is worth loading again. A missing `dir` is 0.
pub fn package_stamp(dir: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let Ok(folders) = package_folders(dir) else {
        return 0;
    };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for folder in folders {
        folder.hash(&mut hasher);
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        let mut files: Vec<(std::ffi::OsString, u64, u128)> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let meta = entry.metadata().ok()?;
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |since| since.as_nanos());
                Some((entry.file_name(), meta.len(), modified))
            })
            .collect();
        files.sort();
        files.hash(&mut hasher);
    }
    // Never the "missing" value by accident.
    hasher.finish().max(1)
}

/// The package folders directly under `dir`, sorted: every folder with an
/// `effect.toml` in it. Anything else there is left alone, so a stray file
/// or a folder of notes beside the packages costs nothing.
pub fn package_folders(dir: &Path) -> Result<Vec<PathBuf>, Error> {
    let entries = std::fs::read_dir(dir).map_err(|error| Error::Io {
        path: dir.to_path_buf(),
        message: error.to_string(),
    })?;
    let mut folders: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join("effect.toml").is_file())
        .collect();
    folders.sort();
    Ok(folders)
}

/// A path as one FFmpeg option value: single-quoted, with the quote
/// itself the one character that has to be spelt out.
fn escape_option(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn check_names(expr: &Expr, known: &[String]) -> Result<(), String> {
    let mut names = Vec::new();
    expr.names(&mut names);
    for name in names {
        if !known.contains(&name) {
            return Err(format!("reads `{name}`, which is not declared"));
        }
    }
    Ok(())
}

/// The catalogue every caller reads; see `Catalogue::builtin`.
static CURRENT: RwLock<Option<&'static Catalogue>> = RwLock::new(None);

/// Every package the app knows.
#[derive(Clone, Debug, Default)]
pub struct Catalogue {
    packages: Vec<Package>,
    by_id: HashMap<String, usize>,
}

impl Catalogue {
    /// An empty catalogue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every package the process knows: the ones compiled into the binary
    /// and, once [`Catalogue::install`] has run, the ones in the user's
    /// folder. Built on first use; a built-in that fails to load is a
    /// build defect, and the tests catch it.
    pub fn builtin() -> &'static Catalogue {
        if let Some(current) = *CURRENT.read().expect("catalogue lock") {
            return current;
        }
        let mut slot = CURRENT.write().expect("catalogue lock");
        if let Some(current) = *slot {
            return current;
        }
        let built: &'static Catalogue = Box::leak(Box::new(Catalogue::compiled_in()));
        *slot = Some(built);
        built
    }

    /// Makes the packages under `dir` part of the catalogue, beside the
    /// built-ins, and reports the ones that would not load. Called at
    /// start and again after an import; each call builds the catalogue
    /// afresh and leaks the last one, which is a few kilobytes a time and
    /// what keeps every caller's `&'static` honest.
    pub fn install(dir: &Path) -> Vec<Error> {
        Self::install_with(dir, &mut |_| Ok(()))
    }

    /// [`Catalogue::install`], with every package the folder holds put to
    /// `vet` before it is offered: one the vet refuses is left out and
    /// reported the way one that failed to parse is. The window's vet runs
    /// a package's shader once on the GPU against a timeout, so a shader
    /// that hangs the device is found at install, not on the first frame
    /// that asks for it (audit 2026-09-23, #7).
    pub fn install_with(
        dir: &Path,
        vet: &mut dyn FnMut(&Package) -> Result<(), String>,
    ) -> Vec<Error> {
        let mut catalogue = Catalogue::compiled_in();
        let mut errors = if dir.is_dir() {
            catalogue.load_dir(dir)
        } else {
            Vec::new()
        };
        let refused: Vec<(String, String)> = catalogue
            .packages
            .iter()
            .filter(|package| package.folder.is_some())
            .filter_map(|package| vet(package).err().map(|why| (package.id().to_owned(), why)))
            .collect();
        for (id, message) in refused {
            catalogue.remove(&id);
            errors.push(Error::Invalid { id, message });
        }
        let built: &'static Catalogue = Box::leak(Box::new(catalogue));
        *CURRENT.write().expect("catalogue lock") = Some(built);
        errors
    }

    /// Takes a package out, by id, and reindexes what is left.
    fn remove(&mut self, id: &str) {
        self.packages.retain(|package| package.id() != id);
        self.by_id.clear();
        for (index, package) in self.packages.iter().enumerate() {
            self.by_id.insert(package.id().to_owned(), index);
            for alias in &package.manifest.effect.aliases {
                self.by_id.insert(alias.clone(), index);
            }
        }
    }

    /// The packages compiled into the binary, and nothing else.
    fn compiled_in() -> Catalogue {
        let mut catalogue = Catalogue::new();
        for (folder, manifest, fixtures, shader) in crate::builtins::BUILTIN_SOURCES {
            let package = Package::from_sources(manifest, *fixtures, *shader)
                .unwrap_or_else(|error| panic!("built-in package {folder}: {error}"));
            assert_eq!(
                package.id(),
                *folder,
                "package folder must be named after its id"
            );
            catalogue
                .add(package)
                .unwrap_or_else(|error| panic!("built-in package {folder}: {error}"));
        }
        catalogue.sort();
        catalogue
    }

    /// Adds a package. Its id and aliases must be new to the catalogue.
    pub fn add(&mut self, package: Package) -> Result<(), Error> {
        let mut names = vec![package.id().to_owned()];
        names.extend(package.manifest.effect.aliases.iter().cloned());
        for name in &names {
            if self.by_id.contains_key(name) {
                return Err(Error::Invalid {
                    id: package.id().to_owned(),
                    message: format!("`{name}` is already taken by another package"),
                });
            }
        }
        let index = self.packages.len();
        for name in names {
            self.by_id.insert(name, index);
        }
        self.packages.push(package);
        Ok(())
    }

    /// Loads every package folder directly under `dir`. A folder that fails
    /// is reported and skipped; the rest still load.
    pub fn load_dir(&mut self, dir: &Path) -> Vec<Error> {
        let folders = match package_folders(dir) {
            Ok(folders) => folders,
            Err(error) => return vec![error],
        };
        let mut errors = Vec::new();
        for folder in folders {
            match Package::from_folder(&folder).and_then(|p| self.add(p)) {
                Ok(()) => {}
                Err(error) => errors.push(error),
            }
        }
        self.sort();
        errors
    }

    fn sort(&mut self) {
        let mut order: Vec<usize> = (0..self.packages.len()).collect();
        order.sort_by(|&a, &b| {
            let (a, b) = (
                &self.packages[a].manifest.effect,
                &self.packages[b].manifest.effect,
            );
            (a.order, a.name.as_str()).cmp(&(b.order, b.name.as_str()))
        });
        let packages: Vec<Package> = order.iter().map(|&i| self.packages[i].clone()).collect();
        self.packages = packages;
        self.by_id.clear();
        for (index, package) in self.packages.iter().enumerate() {
            self.by_id.insert(package.id().to_owned(), index);
            for alias in &package.manifest.effect.aliases {
                self.by_id.insert(alias.clone(), index);
            }
        }
    }

    /// The package with this id or alias.
    pub fn get(&self, id: &str) -> Option<&Package> {
        self.by_id.get(id).map(|&index| &self.packages[index])
    }

    /// Every package, in catalogue order.
    pub fn packages(&self) -> impl Iterator<Item = &Package> {
        self.packages.iter()
    }

    /// Every package of one kind, in catalogue order.
    pub fn of_kind(&self, kind: Kind) -> impl Iterator<Item = &Package> {
        self.packages
            .iter()
            .filter(move |package| package.kind() == kind)
    }

    /// Every package of one kind matching a category filter, in catalogue order.
    pub fn of_category<'a>(
        &'a self,
        kind: Kind,
        category: &'a str,
    ) -> impl Iterator<Item = &'a Package> {
        self.of_kind(kind)
            .filter(move |package| package.matches_category(category))
    }

    /// The complete FFmpeg video filter string for a clip's effects, or the
    /// empty string if it has none. Effects apply in the order they were
    /// added.
    pub fn video_chain(&self, effects: &[AppliedFilter]) -> String {
        self.compose(&[Kind::Effect, Kind::Filter], effects, false)
    }

    /// The video chain for a renderer that runs shaders: every package with
    /// one is left out, because [`Catalogue::shader_passes`] carries it.
    pub fn video_chain_gpu(&self, effects: &[AppliedFilter]) -> String {
        self.compose(&[Kind::Effect, Kind::Filter], effects, true)
    }

    /// The shader passes of a clip's chain, in applied order: one per enabled
    /// entry whose package has a shader. A filter's intensity rides along;
    /// an effect is always whole. `reveal_map` is a title's baked per-word
    /// order, carried to every pass exactly as a package's own look-up
    /// table is - None for anything that is not a title, harmless for a
    /// package that never reads `reveal_order()`.
    pub fn shader_passes(
        &self,
        effects: &[AppliedFilter],
        reveal_map: Option<Arc<RevealMap>>,
    ) -> Vec<ShaderPass> {
        self.passes_with(effects, |applied| applied.params.clone(), reveal_map)
    }

    /// The same passes at one instant of the clip, `at` in `0..=1`: a
    /// parameter with keys is worth what its ride says there. What a
    /// renderer asks for each frame of a clip whose chain rides.
    pub fn shader_passes_at(
        &self,
        effects: &[AppliedFilter],
        at: f64,
        reveal_map: Option<Arc<RevealMap>>,
    ) -> Vec<ShaderPass> {
        self.passes_with(effects, |applied| applied.params_at(at), reveal_map)
    }

    /// The two-input pass for a transition package at `progress` in `0..=1`,
    /// or None when `id` names no transition this catalogue knows (an unknown
    /// id, or one that resolves to an effect/filter). The renderer combines
    /// the outgoing and incoming pictures with it.
    pub fn transition_pass(
        &self,
        id: &str,
        params: &BTreeMap<String, f64>,
        progress: f64,
    ) -> Option<TransitionPass> {
        let package = self.get(id)?;
        let shader = package.transition()?;
        let values = package.resolve(params);
        Some(shader.pass(
            &values,
            &package.manifest.params,
            progress.clamp(0.0, 1.0) as f32,
            package.lut.clone(),
        ))
    }

    fn passes_with(
        &self,
        effects: &[AppliedFilter],
        params_of: impl Fn(&AppliedFilter) -> BTreeMap<String, f64>,
        reveal_map: Option<Arc<RevealMap>>,
    ) -> Vec<ShaderPass> {
        effects
            .iter()
            .filter(|applied| applied.enabled)
            .filter_map(|applied| {
                let package = self.get(&applied.id)?;
                let shader = package.shader()?;
                let set = params_of(applied);
                let intensity = if package.kind() == Kind::Filter {
                    (set.get(INTENSITY).copied().unwrap_or(100.0) / 100.0).clamp(0.0, 1.0) as f32
                } else {
                    1.0
                };
                let values = package.resolve(&set);
                Some(shader.pass(
                    &values,
                    &package.manifest.params,
                    intensity,
                    package.lut.clone(),
                    reveal_map.clone(),
                ))
            })
            .collect()
    }

    /// The complete FFmpeg audio filter string for a clip's filters, or the
    /// empty string if it has none. Filters apply in the order they were
    /// added: EQ before a limiter is a different sound from the reverse.
    pub fn audio_chain(&self, filters: &[AppliedFilter]) -> String {
        self.compose(&[Kind::Audio], filters, false)
    }

    /// Enabled entries of these `kinds` in applied order, comma-joined. Bypassed
    /// entries, unknown ids, packages of another kind and packages without
    /// an FFmpeg backend contribute nothing. The index each fragment is
    /// rendered at is its *emitted* position, so labels stay stable when a
    /// bypassed entry sits earlier in the list.
    fn compose(&self, kinds: &[Kind], applied: &[AppliedFilter], skip_shaders: bool) -> String {
        let mut fragments: Vec<String> = Vec::new();
        for applied in applied.iter().filter(|applied| applied.enabled) {
            let Some(package) = self.get(&applied.id) else {
                continue;
            };
            if !kinds.contains(&package.kind()) {
                continue;
            }
            if skip_shaders && package.shader().is_some() {
                continue;
            }
            match package.ffmpeg_fragment(&applied.params, fragments.len()) {
                Ok(Some(fragment)) => {
                    let index = fragments.len();
                    fragments.push(mixed(package, &applied.params, fragment, index));
                }
                Ok(None) => {}
                // Every template was rendered at load; a failure here is a
                // package whose expression only breaks for some value. Drop
                // the fragment rather than export a broken graph.
                Err(_) => {}
            }
        }
        fragments.join(",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chain reads the frame and its own table and no other file.
    #[test]
    fn a_chain_that_names_a_file_is_refused() {
        assert_eq!(names_a_file("lut3d=file={lut}:interp=tetrahedral"), None);
        assert_eq!(names_a_file("hue=h=10,eq=brightness=0.1"), None);
        assert_eq!(names_a_file("lut3d=file=/etc/passwd"), Some("file="));
        assert_eq!(names_a_file("curves=psfile=/tmp/x.acv"), Some("psfile="));
        assert_eq!(names_a_file("lut1d=filename=x.cube"), Some("filename="));
        assert_eq!(
            names_a_file("scale=profile=1"),
            None,
            "an option that merely ends in the word"
        );
    }
}
