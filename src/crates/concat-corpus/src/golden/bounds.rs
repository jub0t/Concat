// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! How far from its reference a picture may measure, and the file that
//! says so: `golden/golden.json` in a case's folder, beside the PNGs it
//! speaks for.
//!
//! There is one set of references, and one set of bounds every golden
//! starts from: [`Bounds::DEFAULT`], sized so that two honest renders of
//! one scene pass and nothing a person would see does (the measurements
//! are with the constant). Where one kind of adapter draws a golden
//! further off than that, and the difference has been looked at and is
//! that adapter's and not the app's, the golden carries an override for
//! that [`Family`] alone:
//!
//! ```json
//! {
//!   "goldens": {
//!     "title": {
//!       "blessedOn": "lavapipe",
//!       "overrides": {
//!         "warp": {
//!           "bounds": { "flipP99": 0.18 },
//!           "measured": { "flipMean": 0.0053, "flipP99": 0.14, "deltaE": 0.61 },
//!           "why": "WARP covers the glyphs' edges in quarters of a pixel"
//!         }
//!       }
//!     }
//!   }
//! }
//! ```
//!
//! An override states the bounds it changes, the numbers that were
//! measured when it was written, and why: all three, or the file does not
//! load. It is read strictly in every other way too. A bound is above
//! nought and not below what was measured; a looser bound than the
//! default's is only taken where the measurement was past the default's,
//! since otherwise there is nothing for it to excuse.
//!
//! The measurement is kept because an override outlives its reason. When
//! what it excused is fixed, the golden goes on passing, the override
//! goes on allowing what no longer happens, and the next real difference
//! that size passes too. So a pass that is far inside what the override
//! measured is flagged as unexpected: see [`Entry::stale`].

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The file's name, in a case's `golden` folder.
pub const FILE: &str = "golden.json";

/// The folder in a case's folder that holds its references.
pub const FOLDER: &str = "golden";

/// The three numbers a picture is held to: none may be passed.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Bounds {
    /// The most FLIP's mean error may be.
    pub flip_mean: f64,
    /// The most FLIP's 99th percentile by weight may be.
    pub flip_p99: f64,
    /// The most any 8 × 8 block's mean colour may be off, in ΔE2000.
    pub delta_e: f64,
}

impl Bounds {
    /// The bounds every golden has unless its family has an override: the
    /// one place they are set.
    ///
    /// They were set from measurements of a made-up 320 × 180 scene (a
    /// gradient, flat fields, discs, a slanted bar, two-pixel stripes),
    /// drawn two ways and compared; `golden::synth`'s `measure` prints
    /// them. What two honest renders of one scene differ by:
    ///
    /// | difference                          | mean   | p99    | ΔE    |
    /// |-------------------------------------|--------|--------|-------|
    /// | truncated instead of rounded        | 0.0178 | 0.0356 | 0.30  |
    /// | dithered by half a level            | 0.0067 | 0.0175 | 0.18  |
    /// | noise of one level, every pixel     | 0.0139 | 0.0335 | 0.29  |
    /// | noise of two levels, every pixel    | 0.0206 | 0.0493 | 0.62  |
    /// | every channel one level up          | 0.0351 | 0.0478 | 0.40  |
    /// | the scene moved 1/256 of a pixel    | 0.0037 | 0.0455 | 0.37  |
    /// | the scene moved 1/64 of a pixel     | 0.0044 | 0.0612 | 0.37  |
    ///
    /// and the worst of each on flat fields of any colour a level out:
    /// FLIP 0.054 (a saturated green, every channel a level up) and ΔE
    /// 2.37 (a dark grey, its channels a level out in opposite
    /// directions, which is the most one level of rounding can cost
    /// anywhere in sRGB). Against those, the least of what a golden is
    /// there to catch:
    ///
    /// | difference                          | mean   | p99    | ΔE    |
    /// |-------------------------------------|--------|--------|-------|
    /// | a field four levels redder          | 0.0060 | 0.0816 | 1.55  |
    /// | an 8 × 8 patch four levels redder   | 0.0001 | 0.0829 | 1.55  |
    /// | the whole 2% dimmer                 | 0.0781 | 0.1511 | 1.09  |
    /// | the scene moved a quarter pixel     | 0.0404 | 0.3293 | 4.04  |
    /// | one white pixel                     | 0.0001 | 0.4142 | 1.38  |
    ///
    /// So each bound sits in the gap: the mean between 0.054 and 0.078,
    /// the percentile between 0.061 and 0.082, and ΔE just past the 2.37
    /// that rounding alone can reach, so that it is never tripped by a
    /// flat field rounded the other way and still is by a field gone off
    /// its hue (6.4 for a tan turned mauve at the same lightness; 6.7 for
    /// one 8 × 8 block of it under two levels of noise, which both of
    /// FLIP's numbers pass, at 0.021 and 0.060). The percentile's gap is
    /// narrow because the two sides meet there: four levels in one
    /// channel of a flat field is about where a change starts to be one.
    /// Two things an adapter may honestly do measure past them
    /// and are left to an override that says so: edges covered in
    /// quarters of a pixel where the reference's are in sixteenths (p99
    /// 0.140) and a scene sampled a sixteenth of a pixel off (p99 0.144).
    pub const DEFAULT: Bounds = Bounds {
        flip_mean: 0.06,
        flip_p99: 0.08,
        delta_e: 2.5,
    };

    /// The three, with the name each goes by in a sentence.
    pub(crate) fn named(&self) -> [(&'static str, f64); 3] {
        [
            ("FLIP's mean", self.flip_mean),
            ("FLIP's 99th percentile", self.flip_p99),
            ("the worst block's ΔE", self.delta_e),
        ]
    }
}

/// What a picture measured against its reference: [`Bounds`]' three
/// numbers, as found.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Measured {
    /// FLIP's mean error.
    pub flip_mean: f64,
    /// FLIP's 99th percentile by weight.
    pub flip_p99: f64,
    /// The worst 8 × 8 block's ΔE2000.
    pub delta_e: f64,
}

impl Measured {
    pub(crate) fn values(&self) -> [f64; 3] {
        [self.flip_mean, self.flip_p99, self.delta_e]
    }

    /// Whether each number is within its bound, in [`Bounds::named`]'s
    /// order.
    pub(crate) fn within(&self, bounds: &Bounds) -> [bool; 3] {
        let bounds = bounds.named();
        let values = self.values();
        [0, 1, 2].map(|at| values[at] <= bounds[at].1)
    }
}

/// The kinds of adapter a golden may be drawn on, as far as its bounds
/// tell them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Family {
    /// Windows' software adapter, which CI's Windows machines draw on.
    Warp,
    /// Mesa's software Vulkan, which CI's Linux machines draw on.
    Lavapipe,
    /// Apple's GPUs, through Metal.
    Metal,
    /// Any other adapter: a graphics card, through Vulkan or Direct3D.
    Gpu,
}

impl Family {
    /// Every family.
    pub const ALL: [Family; 4] = [Family::Warp, Family::Lavapipe, Family::Metal, Family::Gpu];

    /// The family's name, as `golden.json` has it.
    pub fn name(self) -> &'static str {
        match self {
            Family::Warp => "warp",
            Family::Lavapipe => "lavapipe",
            Family::Metal => "metal",
            Family::Gpu => "gpu",
        }
    }

    /// The family of the adapter wgpu names `adapter` on `backend`
    /// (`AdapterInfo`'s `name` and `backend`, in any case).
    pub fn of_adapter(backend: &str, adapter: &str) -> Family {
        let adapter = adapter.to_ascii_lowercase();
        let backend = backend.to_ascii_lowercase();
        if adapter.contains("llvmpipe") || adapter.contains("lavapipe") {
            Family::Lavapipe
        } else if adapter.contains("microsoft basic render") || adapter.contains("warp") {
            Family::Warp
        } else if backend.contains("metal") {
            Family::Metal
        } else {
            Family::Gpu
        }
    }
}

impl fmt::Display for Family {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl std::str::FromStr for Family {
    type Err = String;

    fn from_str(name: &str) -> Result<Self, String> {
        Family::ALL
            .into_iter()
            .find(|family| family.name() == name)
            .ok_or_else(|| format!("{name} is not a family: warp, lavapipe, metal or gpu"))
    }
}

/// The bounds an override changes: those it names, and no others.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Changed {
    /// See [`Bounds::flip_mean`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flip_mean: Option<f64>,
    /// See [`Bounds::flip_p99`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flip_p99: Option<f64>,
    /// See [`Bounds::delta_e`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta_e: Option<f64>,
}

impl Changed {
    fn values(&self) -> [Option<f64>; 3] {
        [self.flip_mean, self.flip_p99, self.delta_e]
    }
}

/// One family's exception for one golden. Every field is required of the
/// file; they are optional here only so that a missing one is refused in
/// this module's words and not the JSON reader's.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Override {
    /// The bounds it changes.
    #[serde(default)]
    pub bounds: Changed,
    /// What the golden measured on this family when the override was
    /// written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measured: Option<Measured>,
    /// Why the family draws it differently, for whoever next wonders
    /// whether it still does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// One golden's entry in the file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Entry {
    /// The family the reference was drawn on when it was last blessed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blessed_on: Option<Family>,
    /// The families held to other bounds than the default.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub overrides: BTreeMap<Family, Override>,
}

/// Where the bounds a picture was held to came from.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "from", rename_all = "camelCase")]
pub enum Source {
    /// [`Bounds::DEFAULT`].
    Default,
    /// The golden's override for the family it was drawn on.
    #[serde(rename_all = "camelCase")]
    Override {
        /// The family.
        family: Family,
        /// The override's reason.
        why: String,
        /// What the override says was measured.
        measured: Measured,
    },
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::Default => f.write_str("the default bounds"),
            Source::Override { family, why, .. } => {
                write!(f, "the bounds of the override for {family}: {why}")
            }
        }
    }
}

/// How far inside what its override measured a number must come before
/// the override is called stale: half. A software adapter draws the same
/// picture every run and a graphics card's driver moves a result by a few
/// percent, so a number that has halved is not noise; something changed,
/// and the override is describing a difference that is no longer there.
pub const STALE: f64 = 0.5;

impl Entry {
    /// The bounds for a picture drawn on `family`, and where they came
    /// from.
    pub fn bounds(&self, family: Family) -> (Bounds, Source) {
        let Some(over) = self.overrides.get(&family) else {
            return (Bounds::DEFAULT, Source::Default);
        };
        let [mean, p99, delta_e] = over.bounds.values();
        let bounds = Bounds {
            flip_mean: mean.unwrap_or(Bounds::DEFAULT.flip_mean),
            flip_p99: p99.unwrap_or(Bounds::DEFAULT.flip_p99),
            delta_e: delta_e.unwrap_or(Bounds::DEFAULT.delta_e),
        };
        // Both are there: the file does not load without them.
        let source = Source::Override {
            family,
            why: over.why.clone().unwrap_or_default(),
            measured: over.measured.unwrap_or(Measured {
                flip_mean: 0.0,
                flip_p99: 0.0,
                delta_e: 0.0,
            }),
        };
        (bounds, source)
    }

    /// Whether `found`, which passed on `family`, shows the family's
    /// override to be out of date, and if so in what words.
    ///
    /// An override loosens a bound because the family measured past the
    /// default. It is stale in a number it loosened when that number now
    /// comes in under the default, so the override is not needed, or
    /// under [`STALE`] of what the override says was measured, so the
    /// override is wider than what it describes. The rule is set against
    /// the recorded measurement and not against the bound, because how
    /// much room a bound leaves over its measurement is its writer's
    /// choice, and the measurement is a fact.
    pub fn stale(&self, family: Family, found: &Measured) -> Option<String> {
        let over = self.overrides.get(&family)?;
        let measured = over.measured?;
        let default = Bounds::DEFAULT.named();
        for (at, bound) in over.bounds.values().into_iter().enumerate() {
            let (what, default) = default[at];
            let Some(bound) = bound.filter(|bound| *bound > default) else {
                continue;
            };
            let (found, measured) = (found.values()[at], measured.values()[at]);
            if found <= default {
                return Some(format!(
                    "{what} is {found:.4}, inside the default bound of {default} without the override for {family} (bound {bound}, measured {measured:.4}): remove the override"
                ));
            }
            if found <= measured * STALE {
                return Some(format!(
                    "{what} is {found:.4}, under half the {measured:.4} the override for {family} measured (bound {bound}): measure again and tighten the override"
                ));
            }
        }
        None
    }
}

/// A case's `golden.json`: an entry for each of its goldens.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Metadata {
    /// The goldens, by name: the PNG's name without its extension.
    #[serde(default)]
    pub goldens: BTreeMap<String, Entry>,
}

impl Metadata {
    /// Where a case's file is.
    pub fn path(case_dir: &Path) -> PathBuf {
        case_dir.join(FOLDER).join(FILE)
    }

    /// Reads a case's file. A case with no file has no overrides, which is
    /// not an error; a file that does not hold to the rules is.
    pub fn load(case_dir: &Path) -> Result<Metadata, String> {
        let path = Self::path(case_dir);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Metadata::default());
            }
            Err(error) => return Err(format!("could not read {}: {error}", path.display())),
        };
        Self::parse(&text).map_err(|error| format!("{}: {error}", path.display()))
    }

    /// Reads the file's text and holds it to the rules.
    pub fn parse(text: &str) -> Result<Metadata, String> {
        let metadata: Metadata = serde_json::from_str(text).map_err(|error| error.to_string())?;
        for (name, entry) in &metadata.goldens {
            check_name(name)?;
            for (family, over) in &entry.overrides {
                check_override(over)
                    .map_err(|error| format!("{name}'s override for {family} {error}"))?;
            }
        }
        Ok(metadata)
    }

    /// The file's text: the same for the same entries, every time.
    pub fn text(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_owned());
        text.push('\n');
        text
    }

    /// Writes a case's file, making its folder.
    pub fn save(&self, case_dir: &Path) -> Result<(), String> {
        let path = Self::path(case_dir);
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)
                .map_err(|error| format!("could not make {}: {error}", folder.display()))?;
        }
        std::fs::write(&path, self.text())
            .map_err(|error| format!("could not write {}: {error}", path.display()))
    }

    /// The entry for `name`: its own, or one with no overrides.
    pub fn entry(&self, name: &str) -> Entry {
        self.goldens.get(name).cloned().unwrap_or_default()
    }
}

/// A golden's name is a file's name and nothing more: letters, digits,
/// dashes, underscores and dots, not starting with a dot. So it cannot
/// name a file outside its case's folder.
pub fn check_name(name: &str) -> Result<(), String> {
    let plain = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if name.is_empty() || !plain || name.starts_with('.') {
        return Err(format!(
            "\"{name}\" is not a golden's name: letters, digits, dashes, underscores and dots, not starting with a dot"
        ));
    }
    Ok(())
}

/// Holds one override to the rules; the error is the rest of a sentence
/// that begins with whose override it is.
fn check_override(over: &Override) -> Result<(), String> {
    let why = over.why.as_deref().unwrap_or_default();
    if why.trim().is_empty() {
        return Err(
            "has no \"why\": an override says why its family draws the golden differently"
                .to_owned(),
        );
    }
    let Some(measured) = over.measured else {
        return Err(
            "has no \"measured\": an override records what was measured when it was written"
                .to_owned(),
        );
    };
    if measured
        .values()
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err("has a \"measured\" that is not three numbers of nought or more".to_owned());
    }
    let changed = over.bounds.values();
    if changed.iter().all(Option::is_none) {
        return Err("changes no bound".to_owned());
    }
    let names = ["flipMean", "flipP99", "deltaE"];
    let default = Bounds::DEFAULT.named();
    for (at, bound) in changed.into_iter().enumerate() {
        let Some(bound) = bound else { continue };
        let (name, measured, default) = (names[at], measured.values()[at], default[at].1);
        if !bound.is_finite() || bound <= 0.0 {
            return Err(format!("sets {name} to {bound}: a bound is above nought"));
        }
        if bound < measured {
            return Err(format!(
                "sets {name} to {bound}, under the {measured} it says was measured"
            ));
        }
        if bound > default && measured <= default {
            return Err(format!(
                "loosens {name} to {bound} though it measured {measured}, inside the default of {default}: there is nothing for it to excuse"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"{
      "goldens": {
        "title": {
          "blessedOn": "lavapipe",
          "overrides": {
            "warp": {
              "bounds": { "flipP99": 0.18 },
              "measured": { "flipMean": 0.0053, "flipP99": 0.14, "deltaE": 0.61 },
              "why": "WARP covers the glyphs' edges in quarters of a pixel"
            }
          }
        }
      }
    }"#;

    fn found(flip_mean: f64, flip_p99: f64, delta_e: f64) -> Measured {
        Measured {
            flip_mean,
            flip_p99,
            delta_e,
        }
    }

    #[test]
    fn an_override_changes_only_the_bounds_it_names_and_only_for_its_family() {
        let metadata = Metadata::parse(GOOD).unwrap();
        let entry = metadata.entry("title");
        assert_eq!(entry.blessed_on, Some(Family::Lavapipe));
        let (bounds, source) = entry.bounds(Family::Warp);
        assert_eq!(bounds.flip_p99, 0.18);
        assert_eq!(bounds.flip_mean, Bounds::DEFAULT.flip_mean);
        assert_eq!(bounds.delta_e, Bounds::DEFAULT.delta_e);
        assert!(
            source.to_string().contains("quarters of a pixel"),
            "{source}"
        );
        for family in [Family::Lavapipe, Family::Metal, Family::Gpu] {
            assert_eq!(entry.bounds(family), (Bounds::DEFAULT, Source::Default));
        }
        // A golden the file does not name has the defaults.
        assert_eq!(
            metadata.entry("other").bounds(Family::Warp),
            (Bounds::DEFAULT, Source::Default)
        );
    }

    #[test]
    fn the_default_bounds_are_above_nought() {
        for (what, bound) in Bounds::DEFAULT.named() {
            assert!(bound > 0.0, "{what}");
        }
    }

    #[test]
    fn the_file_is_written_as_it_is_read() {
        let metadata = Metadata::parse(GOOD).unwrap();
        let text = metadata.text();
        assert_eq!(Metadata::parse(&text).unwrap(), metadata);
        assert_eq!(Metadata::parse(&text).unwrap().text(), text);
        assert!(text.contains("\"blessedOn\": \"lavapipe\""), "{text}");
        assert!(text.ends_with("}\n"));
    }

    /// `GOOD` with its override replaced by `over`.
    fn with_override(over: &str) -> Result<Metadata, String> {
        Metadata::parse(&format!(
            r#"{{ "goldens": {{ "title": {{ "overrides": {{ "warp": {over} }} }} }} }}"#
        ))
    }

    #[test]
    fn an_override_without_a_why_is_rejected_at_load() {
        let measured = r#""measured": { "flipMean": 0.005, "flipP99": 0.14, "deltaE": 0.6 }"#;
        for why in ["", r#", "why": """#, r#", "why": "   ""#] {
            let error = with_override(&format!(
                r#"{{ "bounds": {{ "flipP99": 0.18 }}, {measured}{why} }}"#
            ))
            .unwrap_err();
            assert!(
                error.contains("title's override for warp has no \"why\""),
                "{error}"
            );
        }
    }

    #[test]
    fn an_override_without_what_was_measured_is_rejected_at_load() {
        let error = with_override(r#"{ "bounds": { "flipP99": 0.18 }, "why": "WARP's edges" }"#)
            .unwrap_err();
        assert!(error.contains("has no \"measured\""), "{error}");
    }

    #[test]
    fn an_override_is_held_to_its_own_measurement_and_to_the_default() {
        let over = |bounds: &str, measured: &str| {
            with_override(&format!(
                r#"{{ "bounds": {bounds}, "measured": {measured}, "why": "WARP's edges" }}"#
            ))
        };
        let measured = r#"{ "flipMean": 0.005, "flipP99": 0.14, "deltaE": 0.6 }"#;
        assert!(over(r#"{ "flipP99": 0.18 }"#, measured).is_ok());
        // A bound of nought, or under what was measured, or none at all.
        let error = over(r#"{ "flipP99": 0 }"#, measured).unwrap_err();
        assert!(error.contains("a bound is above nought"), "{error}");
        let error = over(r#"{ "flipP99": 0.1 }"#, measured).unwrap_err();
        assert!(
            error.contains("under the 0.14 it says was measured"),
            "{error}"
        );
        let error = over("{}", measured).unwrap_err();
        assert!(error.contains("changes no bound"), "{error}");
        // Looser than the default with nothing measured past the default.
        let error = over(r#"{ "deltaE": 4 }"#, measured).unwrap_err();
        assert!(error.contains("nothing for it to excuse"), "{error}");
        // Tighter than the default is allowed: it excuses nothing.
        assert!(over(r#"{ "flipP99": 0.18, "deltaE": 1 }"#, measured).is_ok());
    }

    #[test]
    fn a_family_or_a_field_the_file_should_not_have_is_rejected() {
        let error = Metadata::parse(
            r#"{ "goldens": { "title": { "overrides": { "swiftshader": {} } } } }"#,
        )
        .unwrap_err();
        assert!(error.contains("swiftshader"), "{error}");
        let error =
            Metadata::parse(r#"{ "goldens": { "title": { "tolerance": 3 } } }"#).unwrap_err();
        assert!(error.contains("tolerance"), "{error}");
        let error = Metadata::parse(r#"{ "goldens": { "../title": {} } }"#).unwrap_err();
        assert!(error.contains("not a golden's name"), "{error}");
    }

    #[test]
    fn an_override_is_stale_once_its_number_is_inside_the_default_or_has_halved() {
        let entry = Metadata::parse(GOOD).unwrap().entry("title");
        // As measured, and a little better: the override is doing its job.
        assert_eq!(entry.stale(Family::Warp, &found(0.005, 0.14, 0.6)), None);
        assert_eq!(entry.stale(Family::Warp, &found(0.005, 0.09, 0.6)), None);
        // Inside the default: not needed.
        let stale = entry.stale(Family::Warp, &found(0.005, 0.07, 0.6)).unwrap();
        assert!(stale.contains("remove the override"), "{stale}");
        // No override for the family, nothing to be stale.
        assert_eq!(entry.stale(Family::Metal, &found(0.0, 0.0, 0.0)), None);

        // An override far past the default: half of what it measured is
        // still outside the default, and is stale all the same.
        let wide = with_override(
            r#"{ "bounds": { "flipP99": 0.95 },
                 "measured": { "flipMean": 0.02, "flipP99": 0.9, "deltaE": 1 },
                 "why": "WARP draws the glow's falloff in bands" }"#,
        )
        .unwrap()
        .entry("title");
        assert_eq!(wide.stale(Family::Warp, &found(0.02, 0.6, 1.0)), None);
        let stale = wide.stale(Family::Warp, &found(0.02, 0.3, 1.0)).unwrap();
        assert!(stale.contains("tighten the override"), "{stale}");
    }

    #[test]
    fn an_adapter_is_put_in_its_family_by_its_name_and_backend() {
        assert_eq!(
            Family::of_adapter("Vulkan", "llvmpipe (LLVM 17.0.6, 256 bits)"),
            Family::Lavapipe
        );
        assert_eq!(
            Family::of_adapter("Dx12", "Microsoft Basic Render Driver"),
            Family::Warp
        );
        assert_eq!(Family::of_adapter("Metal", "Apple M2"), Family::Metal);
        assert_eq!(
            Family::of_adapter("Vulkan", "NVIDIA GeForce RTX 3060"),
            Family::Gpu
        );
        assert_eq!(
            Family::of_adapter("Dx12", "Intel(R) HD Graphics 4000"),
            Family::Gpu
        );
        for family in Family::ALL {
            assert_eq!(family.name().parse::<Family>(), Ok(family));
        }
        assert!("opengl".parse::<Family>().is_err());
    }
}
