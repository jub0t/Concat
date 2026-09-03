// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Packages, and the catalogue that holds them.
//!
//! A [`Package`] is a manifest compiled for use: its templates parsed, its
//! `let` bindings checked, its fixtures loaded. A [`Catalogue`] is every
//! package the app knows, findable by id or alias, and it is what turns a
//! clip's applied effects into one FFmpeg chain.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::OnceLock;

use concat_project::model::AppliedFilter;
use serde::Deserialize;

use crate::Error;
use crate::expr::{Expr, Value};
use crate::manifest::{Kind, Manifest};
use crate::template::Template;

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
    /// The pinned outputs shipped with the package.
    pub fixtures: Vec<Fixture>,
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
    /// one, its fixtures file.
    pub fn from_sources(manifest: &str, fixtures: Option<&str>) -> Result<Package, Error> {
        let manifest = Manifest::parse(manifest)?;
        let invalid = |message: String| Error::Invalid {
            id: manifest.effect.id.clone(),
            message,
        };

        let chain = match &manifest.ffmpeg {
            None => None,
            Some(ffmpeg) => {
                let mut known: Vec<String> =
                    manifest.params.iter().map(|p| p.key.clone()).collect();
                known.push("index".to_owned());
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

        let package = Package {
            manifest,
            chain,
            fixtures,
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

    /// Which catalogue the package belongs to.
    pub fn kind(&self) -> Kind {
        self.manifest.effect.kind
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

    /// Runs every fixture; returns one line per failure.
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

    /// The packages compiled into the binary. Loaded once; a built-in that
    /// fails to load is a build defect, and the tests catch it.
    pub fn builtin() -> &'static Catalogue {
        static BUILTIN: OnceLock<Catalogue> = OnceLock::new();
        BUILTIN.get_or_init(|| {
            let mut catalogue = Catalogue::new();
            for (folder, manifest, fixtures) in crate::builtins::BUILTIN_SOURCES {
                let package = Package::from_sources(manifest, *fixtures)
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
        })
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
        let mut errors = Vec::new();
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) => {
                return vec![Error::Io {
                    path: dir.to_path_buf(),
                    message: error.to_string(),
                }];
            }
        };
        let mut folders: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.join("effect.toml").is_file())
            .collect();
        folders.sort();
        for folder in folders {
            let read = |name: &str| std::fs::read_to_string(folder.join(name));
            let manifest = match read("effect.toml") {
                Ok(text) => text,
                Err(error) => {
                    errors.push(Error::Io {
                        path: folder.join("effect.toml"),
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            let fixtures = read("fixtures.toml").ok();
            match Package::from_sources(&manifest, fixtures.as_deref()).and_then(|p| self.add(p)) {
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

    /// The complete FFmpeg video filter string for a clip's effects, or the
    /// empty string if it has none. Effects apply in the order they were
    /// added.
    pub fn video_chain(&self, effects: &[AppliedFilter]) -> String {
        self.compose(&[Kind::Effect, Kind::Filter], effects)
    }

    /// The complete FFmpeg audio filter string for a clip's filters, or the
    /// empty string if it has none. Filters apply in the order they were
    /// added: EQ before a limiter is a different sound from the reverse.
    pub fn audio_chain(&self, filters: &[AppliedFilter]) -> String {
        self.compose(&[Kind::Audio], filters)
    }

    /// Enabled entries of these `kinds` in applied order, comma-joined. Bypassed
    /// entries, unknown ids, packages of another kind and packages without
    /// an FFmpeg backend contribute nothing. The index each fragment is
    /// rendered at is its *emitted* position, so labels stay stable when a
    /// bypassed entry sits earlier in the list.
    fn compose(&self, kinds: &[Kind], applied: &[AppliedFilter]) -> String {
        let mut fragments: Vec<String> = Vec::new();
        for applied in applied.iter().filter(|applied| applied.enabled) {
            let Some(package) = self.get(&applied.id) else {
                continue;
            };
            if !kinds.contains(&package.kind()) {
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
