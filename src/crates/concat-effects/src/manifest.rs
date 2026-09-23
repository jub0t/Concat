// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The manifest: what a package declares about itself.
//!
//! `effect.toml` names the package, lists its parameters, and carries one
//! backend - an FFmpeg chain template, or a WGSL shader. The parameters are
//! the whole of what the window shows a user; the backend is the whole of
//! what the engine runs. A manifest that declares a parameter its backend
//! never reads, or reads one it never declares, is rejected at load.

use serde::Deserialize;

use crate::Error;

/// The package format this build reads: the shape of the manifest, the
/// template and expression syntax, and the shader's contract with the
/// compositor. Bumped when any of those changes in a way an older build
/// could not read, so a package written for a newer Concat says so rather
/// than failing in the shader compiler.
pub const FORMAT: u32 = 1;

/// A parsed `effect.toml`.
#[derive(Deserialize, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// The package format the file is written for, `format = 1` at the top
    /// of the file, before the tables. Absent means 1, the first; see
    /// [`FORMAT`] for the one this build reads.
    #[serde(default = "one")]
    pub format: u32,
    /// Identity and placement.
    pub effect: Meta,
    /// The knobs, in the order the inspector shows them.
    #[serde(default, rename = "param")]
    pub params: Vec<Param>,
    /// The FFmpeg backend, when the package is a filter chain.
    #[serde(default)]
    pub ffmpeg: Option<Ffmpeg>,
    /// The WGSL backend, when the package is a shader.
    #[serde(default)]
    pub wgsl: Option<Wgsl>,
    /// The two-input backend, when the package is a transition.
    #[serde(default)]
    pub transition: Option<Transition>,
    /// A look-up table the package ships: the shader reads it through
    /// `lut()`, and a chain names its file as `{lut}`.
    #[serde(default)]
    pub lut: Option<LutTable>,
}

/// The `[lut]` table: a `.cube` file beside the manifest.
#[derive(Deserialize, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct LutTable {
    /// The file, beside the manifest. Only `.cube` is read.
    pub file: String,
}

/// The `[effect]` table.
#[derive(Deserialize, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Meta {
    /// Namespaced, `author.name`, lower-case. Written into project files,
    /// so it is forever.
    pub id: String,
    /// What the catalogue card says.
    pub name: String,
    /// Which catalogue the package belongs to.
    pub kind: Kind,
    /// The shelf the card sits on, e.g. "Blur". Free text.
    #[serde(default)]
    pub category: String,
    /// The package's own version, bumped when its output changes.
    #[serde(default = "one")]
    pub version: u32,
    /// The parameter the simple view shows as the one slider.
    #[serde(default)]
    pub intensity: Option<String>,
    /// Earlier ids this package answers to, so projects that stored them
    /// keep rendering.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Sort key within the category; ties break on name.
    #[serde(default)]
    pub order: i32,
    /// A sentence for the tooltip.
    #[serde(default)]
    pub description: String,
}

fn one() -> u32 {
    1
}

/// Which catalogue a package belongs to. The vocabulary is the one video
/// editors' users already know: an effect is something that *happens* to
/// the picture, a filter is a colour look with an intensity, and audio is
/// its own world.
#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A video effect: image in, image out. Light, blur, distortion,
    /// texture, motion.
    Effect,
    /// A colour look: image in, image out, one intensity. Warm, mono,
    /// film, cinematic.
    Filter,
    /// An audio effect: sound in, sound out. Voices, tone, space.
    Audio,
    /// A transition: two images and a progress, one image out.
    Transition,
    /// A generator: no input, an image out.
    Generator,
}

impl Kind {
    /// Whether the package works on the picture: effects, filters,
    /// transitions and generators do; audio does not.
    pub fn is_visual(self) -> bool {
        self != Kind::Audio
    }
}

/// One `[[param]]`.
#[derive(Deserialize, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Param {
    /// The name the backend reads and the document stores.
    pub key: String,
    /// What the control is labelled.
    pub label: String,
    /// The subhead the control sits under in a panel that groups its rows -
    /// Color, Lightness, Effects on the Adjust tab. Consecutive params with
    /// the same group share one; empty means none.
    #[serde(default)]
    pub group: String,
    /// What kind of control, and how the number is interpreted.
    #[serde(default, rename = "type")]
    pub kind: ParamType,
    /// Lowest value.
    #[serde(default)]
    pub min: f64,
    /// Highest value.
    #[serde(default = "unit")]
    pub max: f64,
    /// The value an untouched control means.
    #[serde(default)]
    pub default: f64,
    /// Slider increment; 0 means continuous.
    #[serde(default)]
    pub step: f64,
    /// Displayed after the number: "%", "dB", "K", "s".
    #[serde(default)]
    pub unit: String,
    /// Whether the control can carry keyframes.
    #[serde(default)]
    pub animate: bool,
    /// For `enum`: the values the document may hold.
    #[serde(default)]
    pub values: Vec<f64>,
    /// For `enum`: what each value is called, in `values` order.
    #[serde(default)]
    pub labels: Vec<String>,
}

fn unit() -> f64 {
    1.0
}

/// The kinds of parameter. The document stores every kind as a number.
#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum ParamType {
    /// A real number on a slider.
    #[default]
    Float,
    /// A whole number on a stepper.
    Int,
    /// A toggle, stored as 0 or 1.
    Bool,
    /// One of `values`, chosen from a list.
    Enum,
    /// A colour, stored as packed RGBA.
    Color,
    /// A position on the picture, stored as two keys `<key>.x` and `<key>.y`
    /// in the 0..1 square.
    Point,
}

/// The `[ffmpeg]` table: a filter chain template.
#[derive(Deserialize, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Ffmpeg {
    /// Named intermediate values, `"name = expression"`, evaluated in order
    /// before the chain. Each may read the parameters and the names before
    /// it.
    #[serde(default, rename = "let")]
    pub lets: Vec<String>,
    /// The chain template: FFmpeg filter syntax with `{expression}` slots.
    pub chain: String,
}

/// The `[wgsl]` table: a shader.
#[derive(Deserialize, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Wgsl {
    /// The shader file, beside the manifest.
    pub entry: String,
    /// Render passes, in order. Empty means one pass to the output.
    #[serde(default, rename = "pass")]
    pub passes: Vec<Pass>,
}

/// The `[transition]` table: a two-input shader that combines the outgoing
/// picture, the incoming one, and a progress into one. A transition's whole
/// backend is here; it never carries an `[ffmpeg]` chain or a `[wgsl]` table,
/// because the per-clip chain machinery those drive does not apply to a cut.
#[derive(Deserialize, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    /// The shader file, beside the manifest. It declares
    /// `fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32>`.
    pub entry: String,
    /// A built-in FFmpeg `xfade` transition name for the GPU-less export
    /// fallback; absent falls back to a plain dissolve.
    #[serde(default)]
    pub xfade: Option<String>,
    /// The legacy transition id to degrade to where the shader cannot run
    /// (the CPU reference, and export without a GPU); absent means
    /// `cross-fade`.
    #[serde(default)]
    pub fallback: Option<String>,
}

/// The FFmpeg `xfade` transition names a `[transition]` may name as its
/// export fallback. Validated at load so a typo is a load error, not a
/// silent hard cut at export.
pub const XFADE_NAMES: &[&str] = &[
    "fade",
    "fadeblack",
    "fadewhite",
    "fadegrays",
    "fadefast",
    "fadeslow",
    "dissolve",
    "pixelize",
    "distance",
    "radial",
    "smoothleft",
    "smoothright",
    "smoothup",
    "smoothdown",
    "circleopen",
    "circleclose",
    "circlecrop",
    "rectcrop",
    "wipeleft",
    "wiperight",
    "wipeup",
    "wipedown",
    "wipetl",
    "wipetr",
    "wipebl",
    "wipebr",
    "slideleft",
    "slideright",
    "slideup",
    "slidedown",
    "vertopen",
    "vertclose",
    "horzopen",
    "horzclose",
    "diagtl",
    "diagtr",
    "diagbl",
    "diagbr",
    "hlslice",
    "hrslice",
    "vuslice",
    "vdslice",
    "hblur",
    "squeezeh",
    "squeezev",
    "zoomin",
    "hlwind",
    "hrwind",
    "vuwind",
    "vdwind",
    "coverleft",
    "coverright",
    "coverup",
    "coverdown",
    "revealleft",
    "revealright",
    "revealup",
    "revealdown",
];

/// One render pass of a WGSL package.
#[derive(Deserialize, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Pass {
    /// The texture the pass writes, readable by later passes. Absent for
    /// the final pass, which writes the output.
    #[serde(default)]
    pub target: Option<String>,
    /// The target's size as an expression over `WIDTH` and `HEIGHT`; absent
    /// means the output size.
    #[serde(default)]
    pub size: Option<String>,
}

fn is_ident(text: &str) -> bool {
    let mut chars = text.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn is_id_segment(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

impl Manifest {
    /// Parses and validates `source`.
    pub fn parse(source: &str) -> Result<Manifest, Error> {
        let manifest: Manifest = toml::from_str(source).map_err(|error| Error::Invalid {
            id: "?".to_owned(),
            message: error.to_string(),
        })?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn invalid(&self, message: impl Into<String>) -> Error {
        Error::Invalid {
            id: self.effect.id.clone(),
            message: message.into(),
        }
    }

    fn validate(&self) -> Result<(), Error> {
        if self.format == 0 {
            return Err(self.invalid("format is 0; the first package format is 1"));
        }
        if self.format > FORMAT {
            return Err(self.invalid(format!(
                "written for package format {}, and this Concat reads up to {FORMAT}: \
                 a newer Concat is needed",
                self.format
            )));
        }
        let id = &self.effect.id;
        let namespaced = id
            .split_once('.')
            .is_some_and(|(author, name)| is_id_segment(author) && is_id_segment(name));
        if !namespaced {
            return Err(
                self.invalid("id must be `author.name`, lower-case letters, digits and hyphens")
            );
        }
        if self.effect.name.trim().is_empty() {
            return Err(self.invalid("name is empty"));
        }
        if let Some(lut) = &self.lut {
            let plain = !lut.file.contains('/') && !lut.file.contains('\\');
            if !plain || !lut.file.to_ascii_lowercase().ends_with(".cube") {
                return Err(self.invalid("[lut] file must be a `.cube` beside the manifest"));
            }
        }
        for alias in &self.effect.aliases {
            if !is_id_segment(alias)
                && !alias
                    .split_once('.')
                    .is_some_and(|(a, n)| is_id_segment(a) && is_id_segment(n))
            {
                return Err(self.invalid(format!("alias `{alias}` is not an id")));
            }
        }
        let mut seen = std::collections::HashSet::new();
        for param in &self.params {
            if !is_ident(&param.key) {
                return Err(self.invalid(format!(
                    "parameter key `{}` must be lower-case letters, digits and underscores",
                    param.key
                )));
            }
            if param.key == "index" {
                return Err(self.invalid("`index` is reserved"));
            }
            // The key every filter answers to without declaring it: how much
            // of the look is applied, read by the catalogue as a percent. A
            // parameter under that name would be read twice, once as itself
            // and once as the mix, and the two would disagree.
            if param.key == "intensity" {
                return Err(
                    self.invalid("`intensity` is reserved: it is the mix every filter takes")
                );
            }
            if !seen.insert(param.key.as_str()) {
                return Err(self.invalid(format!("parameter `{}` is declared twice", param.key)));
            }
            if param.label.trim().is_empty() {
                return Err(self.invalid(format!("parameter `{}` has no label", param.key)));
            }
            if param.min > param.max {
                return Err(self.invalid(format!("parameter `{}`: min is above max", param.key)));
            }
            if param.default < param.min || param.default > param.max {
                return Err(self.invalid(format!(
                    "parameter `{}`: default {} is outside {}..{}",
                    param.key, param.default, param.min, param.max
                )));
            }
            if param.kind == ParamType::Enum {
                if param.values.is_empty() {
                    return Err(self.invalid(format!("enum `{}` lists no values", param.key)));
                }
                if param.values.len() != param.labels.len() {
                    return Err(self.invalid(format!(
                        "enum `{}` has {} values and {} labels",
                        param.key,
                        param.values.len(),
                        param.labels.len()
                    )));
                }
            }
        }
        if let Some(intensity) = &self.effect.intensity
            && !self.params.iter().any(|param| &param.key == intensity)
        {
            return Err(self.invalid(format!(
                "intensity names `{intensity}`, which is not a parameter"
            )));
        }
        if self.effect.kind == Kind::Transition {
            // A transition's whole backend is its two-input shader; the
            // per-clip chain and single-input shader machinery do not apply.
            let Some(transition) = &self.transition else {
                return Err(self.invalid("a transition needs a [transition] table"));
            };
            if self.ffmpeg.is_some() || self.wgsl.is_some() {
                return Err(
                    self.invalid("a transition's backend is [transition], not [ffmpeg] or [wgsl]")
                );
            }
            if let Some(xfade) = &transition.xfade
                && !XFADE_NAMES.contains(&xfade.as_str())
            {
                return Err(self.invalid(format!(
                    "[transition] xfade `{xfade}` is not a known FFmpeg xfade name"
                )));
            }
        } else {
            if self.transition.is_some() {
                return Err(self.invalid("[transition] is only for a transition package"));
            }
            // Both backends may be present: the shader renders wherever there
            // is a GPU, and the chain is what a machine without one gets.
            if self.ffmpeg.is_none() && self.wgsl.is_none() {
                return Err(self.invalid("no backend: add an [ffmpeg] or a [wgsl] table"));
            }
            if self.ffmpeg.is_some() && self.effect.kind == Kind::Generator {
                return Err(
                    self.invalid("an [ffmpeg] package must be an effect, a filter or audio")
                );
            }
            if self.wgsl.is_some() && self.effect.kind == Kind::Audio {
                return Err(self.invalid("a [wgsl] package cannot be audio"));
            }
        }
        Ok(())
    }

    /// The declared parameter with this key.
    pub fn param(&self, key: &str) -> Option<&Param> {
        self.params.iter().find(|param| param.key == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
        [effect]
        id = "concat.blur"
        name = "Blur"
        kind = "effect"
        category = "Blur"
        intensity = "radius"
        aliases = ["blur"]

        [[param]]
        key = "radius"
        label = "Radius"
        min = 1
        max = 50
        default = 10
        unit = "px"

        [ffmpeg]
        chain = "gblur=sigma={fixed(radius, 1)}"
    "#;

    #[test]
    fn a_good_manifest_parses() {
        let manifest = Manifest::parse(GOOD).expect("parses");
        assert_eq!(manifest.effect.id, "concat.blur");
        assert_eq!(manifest.effect.version, 1);
        assert_eq!(manifest.params[0].kind, ParamType::Float);
        assert_eq!(manifest.params[0].step, 0.0);
        assert!(manifest.ffmpeg.is_some());
    }

    fn rejects(source: &str, needle: &str) {
        let error = Manifest::parse(source).expect_err("rejected").to_string();
        assert!(error.contains(needle), "{error}");
    }

    #[test]
    fn bad_manifests_are_rejected_with_a_reason() {
        rejects(&GOOD.replace("concat.blur", "blur"), "author.name");
        rejects(&GOOD.replace("concat.blur", "Concat.Blur"), "author.name");
        rejects(
            &GOOD.replace("intensity = \"radius\"", "intensity = \"sigma\""),
            "intensity",
        );
        rejects(&GOOD.replace("default = 10", "default = 99"), "outside");
        rejects(
            &GOOD.replace(
                "[ffmpeg]\n        chain = \"gblur=sigma={fixed(radius, 1)}\"",
                "",
            ),
            "no backend",
        );
        rejects(
            &GOOD.replace("key = \"radius\"", "key = \"index\""),
            "reserved",
        );
        rejects(
            &GOOD
                .replace("key = \"radius\"", "key = \"intensity\"")
                .replace("intensity = \"radius\"", "intensity = \"intensity\""),
            "reserved",
        );
        rejects(
            &GOOD.replace("unit = \"px\"", "units = \"px\""),
            "unknown field",
        );
        rejects(
            &GOOD.replace("kind = \"effect\"", "kind = \"transition\""),
            "a transition needs a [transition] table",
        );
    }

    const GOOD_TRANSITION: &str = r#"
        [effect]
        id = "concat.dissolve"
        name = "Dissolve"
        kind = "transition"
        aliases = ["cross-fade"]

        [transition]
        entry = "effect.wgsl"
        xfade = "fade"
    "#;

    #[test]
    fn the_format_is_the_first_thing_read() {
        assert_eq!(Manifest::parse(GOOD).expect("parses").format, 1);
        assert_eq!(
            Manifest::parse(&format!("format = {FORMAT}\n{GOOD}"))
                .expect("parses")
                .format,
            FORMAT
        );
        rejects(&format!("format = {}\n{GOOD}", FORMAT + 1), "newer Concat");
        rejects(&format!("format = 0\n{GOOD}"), "format is 0");
        rejects(&format!("format = \"1\"\n{GOOD}"), "format");
    }

    #[test]
    fn a_transition_manifest_parses() {
        let manifest = Manifest::parse(GOOD_TRANSITION).expect("parses");
        assert_eq!(manifest.effect.kind, Kind::Transition);
        let transition = manifest.transition.expect("a [transition] table");
        assert_eq!(transition.entry, "effect.wgsl");
        assert_eq!(transition.xfade.as_deref(), Some("fade"));
    }

    #[test]
    fn bad_transition_manifests_are_rejected() {
        // A transition may not also carry a per-clip backend.
        rejects(
            &GOOD_TRANSITION.replace(
                "[transition]\n        entry = \"effect.wgsl\"\n        xfade = \"fade\"",
                "[transition]\n        entry = \"effect.wgsl\"\n\n        [ffmpeg]\n        chain = \"null\"",
            ),
            "not [ffmpeg] or [wgsl]",
        );
        // An unknown xfade name is a typo caught at load.
        rejects(
            &GOOD_TRANSITION.replace("xfade = \"fade\"", "xfade = \"nonsense\""),
            "not a known FFmpeg xfade name",
        );
        // [transition] belongs only to a transition package.
        rejects(
            &GOOD.replace(
                "[ffmpeg]\n        chain = \"gblur=sigma={fixed(radius, 1)}\"",
                "[transition]\n        entry = \"effect.wgsl\"",
            ),
            "only for a transition package",
        );
    }

    #[test]
    fn enum_parameters_pair_values_with_labels() {
        let source = GOOD.replace(
            "unit = \"px\"",
            "type = \"enum\"\n        values = [1, 2]\n        labels = [\"One\"]",
        );
        rejects(&source, "labels");
    }
}
