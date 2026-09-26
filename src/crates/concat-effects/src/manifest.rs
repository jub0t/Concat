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
///
/// Format 2 is the scene-linear contract: the shader samples the working
/// space as it is - linear light, unclipped, 1.0 the white of an SDR
/// picture - and a package that draws the picture runs on the GPU alone,
/// with no `[ffmpeg]` chain. Format 1 packages still load, and their
/// shaders are handed the gamma-encoded `0..1` picture they were written
/// for (see `concat-effects/src/shader.rs`).
pub const FORMAT: u32 = 2;

/// The first format whose shaders sample the working space as it is.
pub const SCENE_LINEAR: u32 = 2;

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
    /// What the package's card in the catalogue shows, where its defaults
    /// alone would show little.
    #[serde(default)]
    pub card: Option<CardSettings>,
}

/// The `[card]` table. A card is the reference picture through the package
/// at its defaults, a little way into a clip; this is for a package whose
/// defaults show little there - an exposure of nothing, a key with no
/// screen to key, a flash that has burned the cut white.
#[derive(Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct CardSettings {
    /// Knob values set over the defaults, within each knob's range; a
    /// filter's `intensity` among them, a point's as `<key>.x`, `<key>.y`.
    #[serde(default)]
    pub params: std::collections::BTreeMap<String, f64>,
    /// Seconds into the clip the card shows, for an effect or a look.
    #[serde(default)]
    pub moment: Option<f64>,
    /// How far through the cut the card shows, `0..=1`, for a transition.
    #[serde(default)]
    pub progress: Option<f64>,
    /// For a key: the reference picture stands before a screen of this
    /// colour, `#rrggbb`, for the key to take out.
    #[serde(default)]
    pub screen: Option<String>,
}

impl CardSettings {
    /// The screen's colour as bytes, when it names one that parses.
    pub fn screen_rgb(&self) -> Option<[u8; 3]> {
        let hex = self.screen.as_deref()?.strip_prefix('#')?;
        if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
        Some([byte(0)?, byte(2)?, byte(4)?])
    }
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
    /// The passes drawn before `effect`, in order, each into a picture of
    /// its own (see [`Pass`]); `effect` is always the last, drawing the
    /// result. Empty - most packages - is `effect` alone. A format 2
    /// setting.
    #[serde(default, rename = "pass")]
    pub passes: Vec<Pass>,
    /// What a format 2 shader works in; see [`Space`].
    #[serde(default)]
    pub space: Space,
}

/// What a format 2 shader's `sample()` hands it, and what its result is
/// read back as.
#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum Space {
    /// The working space as it is: linear light, 1.0 the white of an SDR
    /// picture. Where light is added, scaled, blurred or keyed.
    #[default]
    Linear,
    /// The display encoding: BT.1886's 2.4 gamma over the working space,
    /// SDR white at 1.0, but extended - a highlight climbs past 1.0 and a
    /// colour outside Rec. 709 keeps its negative channel - and nothing
    /// clipped. Where a colour look drawn by eye on an SDR picture reads as
    /// it was drawn; its result is taken back into light and mixed there.
    Display,
    /// Log: ACEScct's curve over the working space, every level of light
    /// from black to far past white between 0 and 1. Where a table made
    /// for ACEScct is read; its result is taken back into light and mixed
    /// there.
    Log,
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

/// One `[[wgsl.pass]]`: a pass drawn before `effect`, into a picture of its
/// own that every pass after it reads.
#[derive(Deserialize, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Pass {
    /// The picture the pass draws, by name. The shader's
    /// `fn <target>(uv: vec2<f32>) -> vec4<f32>` draws it, and the passes
    /// after it read it through `<target>_at(uv)`.
    pub target: String,
    /// How many times smaller than the layer the picture is, across and
    /// down: two expressions over the knobs, each rounded down to a power
    /// of two from 1 to [`MAX_SHRINK`]. Absent is the layer's own size.
    #[serde(default)]
    pub shrink: Option<[String; 2]>,
}

/// The most passes a package may draw before `effect`: each is a picture
/// bound beside the layer, and a pass may bind sixteen.
pub const MAX_PASSES: usize = 8;

/// The most times smaller than the layer a pass's picture may be, across
/// or down.
pub const MAX_SHRINK: u32 = 64;

/// Names a pass's picture may not have: the last pass's function, and the
/// entry point that draws it.
const TAKEN_TARGETS: &[&str] = &["effect", "main"];

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
        if let Some(card) = &self.card {
            self.validate_card(card)?;
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
            if let Some(wgsl) = &self.wgsl
                && wgsl.space != Space::Linear
                && !self.scene_linear()
            {
                return Err(self.invalid(format!(
                    "[wgsl] space is a format {SCENE_LINEAR} setting: a format 1 shader \
                     is handed the gamma-encoded 0..1 picture already"
                )));
            }
            if let Some(wgsl) = &self.wgsl {
                self.validate_passes(&wgsl.passes)?;
            }
            // The picture is drawn on the GPU, in light, and nowhere else:
            // a chain would run on eight bits the shader never sees.
            if self.scene_linear() && self.effect.kind.is_visual() && self.ffmpeg.is_some() {
                return Err(self.invalid(format!(
                    "a format {SCENE_LINEAR} package draws the picture on the GPU alone: \
                     drop the [ffmpeg] table"
                )));
            }
        }
        Ok(())
    }

    /// Whether the package's shader samples the working space as it is,
    /// rather than the gamma-encoded `0..1` a format 1 shader is given.
    pub fn scene_linear(&self) -> bool {
        self.format >= SCENE_LINEAR
    }

    /// The passes before `effect` name their pictures once each, with
    /// names a shader can spell and the host has not taken, a few of them,
    /// and only from format 2. What their functions read, and what their
    /// shrinks say, is the shader's to check (`crate::shader`).
    fn validate_passes(&self, passes: &[Pass]) -> Result<(), Error> {
        if passes.is_empty() {
            return Ok(());
        }
        if !self.scene_linear() {
            return Err(self.invalid(format!("[[wgsl.pass]] is a format {SCENE_LINEAR} setting")));
        }
        if passes.len() > MAX_PASSES {
            return Err(self.invalid(format!(
                "{} passes before `effect`; a package may draw {MAX_PASSES}",
                passes.len()
            )));
        }
        let mut seen = std::collections::HashSet::new();
        for pass in passes {
            let target = pass.target.as_str();
            if !is_ident(target) || !target.starts_with(|c: char| c.is_ascii_lowercase()) {
                return Err(self.invalid(format!(
                    "pass target `{target}` must be a lower-case letter, then letters, digits \
                     and underscores"
                )));
            }
            if TAKEN_TARGETS.contains(&target) {
                return Err(self.invalid(format!(
                    "pass target `{target}` is taken: `effect` is the last pass"
                )));
            }
            if !seen.insert(target) {
                return Err(self.invalid(format!("pass target `{target}` is drawn twice")));
            }
        }
        Ok(())
    }

    /// A `[card]` table sets only knobs the package has, within their
    /// ranges, and places the card by what kind of package it is.
    fn validate_card(&self, card: &CardSettings) -> Result<(), Error> {
        if !self.effect.kind.is_visual() {
            return Err(self.invalid("[card] is for a package that draws the picture"));
        }
        for (key, value) in &card.params {
            let point = key
                .split_once('.')
                .filter(|(_, axis)| *axis == "x" || *axis == "y")
                .and_then(|(name, _)| self.param(name))
                .filter(|param| param.kind == ParamType::Point);
            let range = match (self.param(key), point) {
                (Some(param), _) => param.min..=param.max,
                (None, Some(_)) => 0.0..=1.0,
                (None, None) if key == "intensity" && self.effect.kind == Kind::Filter => {
                    0.0..=100.0
                }
                (None, None) => {
                    return Err(
                        self.invalid(format!("[card] sets `{key}`, which is not a parameter"))
                    );
                }
            };
            if !range.contains(value) {
                return Err(self.invalid(format!(
                    "[card] sets `{key}` to {value}, outside {}..{}",
                    range.start(),
                    range.end()
                )));
            }
        }
        let transition = self.effect.kind == Kind::Transition;
        if let Some(progress) = card.progress {
            if !transition {
                return Err(self.invalid(
                    "[card] progress places a transition's card; an effect's is placed by moment",
                ));
            }
            if !(0.0..=1.0).contains(&progress) {
                return Err(self.invalid("[card] progress is outside 0..1"));
            }
        }
        if let Some(moment) = card.moment {
            if transition {
                return Err(self.invalid(
                    "[card] moment places an effect's card; a transition's is placed by progress",
                ));
            }
            if !(0.0..=60.0).contains(&moment) {
                return Err(self.invalid("[card] moment is outside 0..60 seconds"));
            }
        }
        if card.screen.is_some() {
            if transition {
                return Err(self.invalid("[card] screen is for an effect, not a transition"));
            }
            if card.screen_rgb().is_none() {
                return Err(self.invalid("[card] screen is not a `#rrggbb` colour"));
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
        assert!(!Manifest::parse(GOOD).expect("parses").scene_linear());
        let shader = GOOD.replace(
            "[ffmpeg]\n        chain = \"gblur=sigma={fixed(radius, 1)}\"",
            "[wgsl]\n        entry = \"effect.wgsl\"",
        );
        let current = Manifest::parse(&format!("format = {FORMAT}\n{shader}")).expect("parses");
        assert_eq!(current.format, FORMAT);
        assert!(current.scene_linear());
        rejects(&format!("format = {}\n{GOOD}", FORMAT + 1), "newer Concat");
        rejects(&format!("format = 0\n{GOOD}"), "format is 0");
        rejects(&format!("format = \"1\"\n{GOOD}"), "format");
    }

    /// A card may set only the package's own knobs, in range, and is
    /// placed by moment for an effect and by progress for a transition.
    #[test]
    fn a_card_table_is_checked_like_the_rest() {
        let with = |card: &str| format!("{GOOD}\n        [card]\n        {card}\n");
        let manifest = Manifest::parse(&with("params = { radius = 20 }\n        moment = 2.5"))
            .expect("parses");
        let card = manifest.card.expect("a card");
        assert_eq!(card.params.get("radius"), Some(&20.0));
        assert_eq!(card.moment, Some(2.5));
        rejects(&with("params = { sigma = 20 }"), "not a parameter");
        rejects(&with("params = { radius = 99 }"), "outside 1..50");
        rejects(&with("progress = 0.5"), "places a transition's card");
        rejects(&with("moment = -1"), "outside 0..60");
        rejects(&with("screen = \"green\""), "#rrggbb");
        let keyed = Manifest::parse(&with("screen = \"#20c040\"")).expect("parses");
        assert_eq!(
            keyed.card.and_then(|card| card.screen_rgb()),
            Some([0x20, 0xc0, 0x40])
        );
        let cut = format!("{GOOD_TRANSITION}\n        [card]\n        progress = 0.2\n");
        assert_eq!(
            Manifest::parse(&cut)
                .expect("parses")
                .card
                .and_then(|c| c.progress),
            Some(0.2)
        );
        rejects(
            &cut.replace("progress = 0.2", "moment = 1"),
            "placed by progress",
        );
        let sound = GOOD.replace("kind = \"effect\"", "kind = \"audio\"");
        rejects(
            &format!("{sound}\n        [card]\n        moment = 1\n"),
            "draws the picture",
        );
    }

    /// A shader may work in the display encoding from format 2, where the
    /// host has one to hand it.
    #[test]
    fn a_display_space_is_a_format_2_setting() {
        let display = GOOD.replace(
            "[ffmpeg]\n        chain = \"gblur=sigma={fixed(radius, 1)}\"",
            "[wgsl]\n        entry = \"effect.wgsl\"\n        space = \"display\"",
        );
        let manifest = Manifest::parse(&format!("format = 2\n{display}")).expect("parses");
        assert_eq!(manifest.wgsl.map(|wgsl| wgsl.space), Some(Space::Display));
        rejects(&display, "format 2 setting");
        let log = display.replace("\"display\"", "\"log\"");
        let manifest = Manifest::parse(&format!("format = 2\n{log}")).expect("parses");
        assert_eq!(manifest.wgsl.map(|wgsl| wgsl.space), Some(Space::Log));
        rejects(&log, "format 2 setting");
        rejects(
            &format!(
                "format = 2\n{}",
                display.replace("\"display\"", "\"gamma\"")
            ),
            "unknown variant",
        );
    }

    /// From format 2 a picture is drawn on the GPU alone, so a chain beside
    /// the shader is refused; sound is FFmpeg's, and keeps its chain.
    #[test]
    fn a_scene_linear_package_draws_without_a_chain() {
        let both = GOOD.replace(
            "[ffmpeg]",
            "[wgsl]\n        entry = \"effect.wgsl\"\n\n        [ffmpeg]",
        );
        Manifest::parse(&format!("format = 1\n{both}")).expect("format 1 may carry both");
        rejects(&format!("format = 2\n{both}"), "drop the [ffmpeg] table");
        rejects(&format!("format = 2\n{GOOD}"), "drop the [ffmpeg] table");
        let audio = GOOD.replace("kind = \"effect\"", "kind = \"audio\"");
        Manifest::parse(&format!("format = 2\n{audio}")).expect("audio keeps its chain");
    }

    /// Passes before `effect` are a format 2 setting; each names a picture
    /// once, a name a shader can spell and the host has not taken, and a
    /// package draws a few of them.
    #[test]
    fn passes_name_their_pictures_once_from_format_2() {
        let shader = GOOD.replace(
            "[ffmpeg]\n        chain = \"gblur=sigma={fixed(radius, 1)}\"",
            "[wgsl]\n        entry = \"effect.wgsl\"",
        );
        let with = |passes: &str| format!("format = 2\n{shader}\n{passes}");
        let pass = |target: &str| format!("[[wgsl.pass]]\ntarget = \"{target}\"\n");
        let manifest = Manifest::parse(&with(&format!(
            "{}shrink = [\"radius / 3\", \"1\"]\n{}",
            pass("across"),
            pass("down")
        )))
        .expect("parses");
        let passes = manifest.wgsl.expect("a shader").passes;
        assert_eq!(passes.len(), 2);
        assert_eq!(
            passes[0].shrink,
            Some(["radius / 3".to_owned(), "1".to_owned()])
        );
        assert_eq!(passes[1].shrink, None);

        rejects(
            &format!("format = 1\n{shader}\n{}", pass("across")),
            "format 2 setting",
        );
        for name in ["Across", "2x", "_x", "a-b", ""] {
            rejects(&with(&pass(name)), "must be a lower-case letter");
        }
        for name in ["effect", "main"] {
            rejects(&with(&pass(name)), "is taken");
        }
        rejects(&with(&format!("{}{}", pass("x"), pass("x"))), "drawn twice");
        let many: String = (0..=MAX_PASSES).map(|n| pass(&format!("p{n}"))).collect();
        rejects(&with(&many), "a package may draw 8");
        rejects(&with(&format!("{}shrink = [\"2\"]\n", pass("x"))), "length");
        rejects(
            &with(&format!("{}size = \"WIDTH / 2\"\n", pass("x"))),
            "unknown field",
        );
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
