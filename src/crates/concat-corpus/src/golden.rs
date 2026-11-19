// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Goldens: a picture the compositor drew, held to a stored reference.
//!
//! Most of what a case asserts needs no stored picture: its media says
//! what it is ([`crate::code`]), and two renders that must be equal are
//! held to each other. A golden is for what is left, the things with no
//! answer but what they look like: how a title is drawn, the fall-off of
//! a mask's feather, an effect's look, a transition's shape. For those a
//! reference is kept, `golden/<name>.png` in the case's folder, and the
//! compositor's own pixels, before any encoder, are compared with it.
//!
//! Two gates, and a picture passes only by both:
//!
//! - [`flip`]: NVIDIA's FLIP, the difference a person flipping between
//!   the two would see, bounded in its mean and in its 99th percentile.
//!   It catches what moved, what appeared and what changed shape.
//! - [`colour`]: the ΔE2000 between the mean colours of each 8 × 8 block,
//!   bounded in the worst block. It catches a flat field gone off its
//!   colour, however small, which FLIP's pooling can lose under the
//!   rounding noise of a whole frame.
//!
//! Before either, the two must be one size and agree in alpha to within
//! [`ALPHA_LEVELS`]; where they are not opaque they are compared as seen
//! over mid grey.
//!
//! The bounds are [`Bounds::DEFAULT`] unless the golden has an override
//! for the adapter [`Family`] it was drawn on; [`bounds`] has the file
//! and its rules. A pass that shows an override to be out of date is
//! reported as an unexpected pass.
//!
//! A reference is never written by a run that was not told to. A golden
//! with no reference fails, saying how to bless it, and nothing is
//! written; a typo in a name is then a failure and not a new reference
//! that passes for ever after. References are written only when
//! `CONCAT_CORPUS_BLESS` names the case (or is `1` or `all`), only those
//! that fail unless `CONCAT_CORPUS_BLESS_FORCE` is set, never when `CI`
//! is; and each one written is listed in a log with what it replaced, to
//! be looked at before it is committed. See [`Mode`].
//!
//! A failure leaves four pictures in the folder the caller names: what
//! was drawn, the reference, their difference scaled up to be seen, and
//! FLIP's map of where a person would see it. [`page`] lays a run's
//! results out as one HTML file with the failures first. [`budget`] keeps
//! the references small enough to live in git, and [`mod@spread`] measures
//! how far apart renders of one frame come, which is what a bound is set
//! from.

pub mod bounds;
pub mod budget;
pub mod colour;
pub mod flip;
pub mod page;
pub mod picture;
pub mod spread;
#[cfg(test)]
mod synth;
#[cfg(test)]
mod tests;

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::Serialize;

pub use bounds::{Bounds, Family, Measured, Metadata, Source};
pub use budget::{Audit, Budget, audit};
pub use page::{failure_page, write_failure_page};
pub use picture::Rgba8Image;
pub use spread::{Spread, spread};

use crate::report::Check;

/// The variable that names the cases whose references may be written:
/// `1` or `all` for every case, or names with commas between.
pub const BLESS: &str = "CONCAT_CORPUS_BLESS";

/// The variable that has a blessing rewrite every reference, not only
/// those that fail.
pub const BLESS_FORCE: &str = "CONCAT_CORPUS_BLESS_FORCE";

/// How many levels a pixel's alpha may be from its reference's: one, a
/// rounding. Nothing the app draws for a golden has been anything but
/// opaque so far; should a golden with a soft clear edge come to differ
/// by more on some adapter, this becomes a bound like the other three.
pub const ALPHA_LEVELS: u8 = 1;

/// Whether a run only compares, or may write references too.
///
/// There is no way to make one that writes but through
/// [`Mode::from_env`] and [`Mode::from_settings`], which refuse where
/// `CI` is set.
#[derive(Clone, Debug)]
pub struct Mode {
    bless: Option<Bless>,
}

#[derive(Clone, Debug)]
struct Bless {
    /// Rewrite references that pass, too.
    force: bool,
    /// The file each reference written is listed in.
    log: PathBuf,
}

impl Mode {
    /// Compare, and write nothing but a failure's pictures.
    pub fn check() -> Mode {
        Mode { bless: None }
    }

    /// The mode the environment asks for, for `case`: [`BLESS`],
    /// [`BLESS_FORCE`] and `CI`. A blessing lists what it wrote in `log`
    /// (`target/corpus/bless.md`).
    pub fn from_env(case: &str, log: &Path) -> Result<Mode, String> {
        let var = |name: &str| std::env::var(name).ok();
        Self::from_settings(
            case,
            log,
            var(BLESS).as_deref(),
            var(BLESS_FORCE).as_deref(),
            var("CI").as_deref(),
        )
    }

    /// [`Mode::from_env`], with the three variables' values handed in.
    ///
    /// A blessing asked for where `ci` is set is an error, whichever case
    /// it names: a reference is a person's judgement that a picture is
    /// right, made on a machine where they can look at it, and a CI run
    /// that could write one could make any failure pass.
    pub fn from_settings(
        case: &str,
        log: &Path,
        bless: Option<&str>,
        force: Option<&str>,
        ci: Option<&str>,
    ) -> Result<Mode, String> {
        let set =
            |value: Option<&str>| value.is_some_and(|v| !matches!(v.trim(), "" | "0" | "false"));
        if !set(bless) {
            return Ok(Mode::check());
        }
        if set(ci) {
            return Err(format!(
                "{BLESS} is set and so is CI: references are blessed on a person's machine and looked at, never in CI"
            ));
        }
        let asked = bless.unwrap_or_default().trim();
        let named = matches!(asked, "1" | "all") || asked.split(',').any(|one| one.trim() == case);
        Ok(Mode {
            bless: named.then(|| Bless {
                force: set(force),
                log: log.to_path_buf(),
            }),
        })
    }

    /// Whether this run may write references.
    pub fn blesses(&self) -> bool {
        self.bless.is_some()
    }
}

/// The pictures a result left for a person to look at.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Artefacts {
    /// What was drawn.
    pub got: Option<PathBuf>,
    /// The reference it was held to: a copy, so that the folder of
    /// artefacts is whole without the repository beside it.
    pub reference: Option<PathBuf>,
    /// Their difference, channel by channel, times
    /// [`Artefacts::difference_gain`].
    pub difference: Option<PathBuf>,
    /// What the difference was multiplied by to be seen.
    pub difference_gain: Option<u32>,
    /// FLIP's error at every pixel, in magma: black the same, pale yellow
    /// as different as can be.
    pub flip: Option<PathBuf>,
}

/// What holding one picture to its golden found.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoldenResult {
    /// The case: its folder's name.
    pub case: String,
    /// The golden.
    pub name: String,
    /// The adapter family the picture was drawn on.
    pub family: Family,
    /// The picture is its reference's to within the bounds, or was
    /// blessed as the reference.
    pub passed: bool,
    /// It passed, and by so much that the override it passed under is
    /// out of date. The detail says how.
    pub unexpected_pass: bool,
    /// The reference was written by this run.
    pub blessed: bool,
    /// What was measured, when there was a reference of the same size to
    /// measure against. On a blessing, against the reference it replaced.
    pub measured: Option<Measured>,
    /// The corner of the 8 × 8 block furthest off its reference's colour.
    pub worst_block: Option<[u32; 2]>,
    /// The bounds it was held to.
    pub bounds: Bounds,
    /// Where those came from.
    pub bounds_from: Source,
    /// Where the reference is, or would be.
    pub reference: PathBuf,
    /// The pictures written: all four on a failure that got as far as
    /// comparing, fewer otherwise, none on a pass.
    pub artefacts: Artefacts,
    /// What was found, in a line.
    pub detail: String,
}

impl GoldenResult {
    /// Nothing here needs a person: it passed, and not unexpectedly.
    pub fn ok(&self) -> bool {
        self.passed && !self.unexpected_pass
    }

    /// The result as a check for a case's [`crate::Report`]. An
    /// unexpected pass is a failed check: left green it would never be
    /// seen, and the override would stay.
    pub fn to_check(&self) -> Check {
        Check {
            name: format!("{} looks as its golden does", self.name),
            passed: self.ok(),
            detail: self.detail.clone(),
        }
    }
}

/// A case's goldens: where they are kept, what they were drawn on, and
/// where a failure's pictures go.
#[derive(Clone, Debug)]
pub struct Golden {
    case_dir: PathBuf,
    case: String,
    family: Family,
    artefacts: PathBuf,
    mode: Mode,
}

/// A picture and its reference, measured.
pub(crate) struct Judged {
    pub measured: Measured,
    pub block: colour::WorstBlock,
    /// The alpha furthest from its reference's: by how many levels, at
    /// which pixel, and how many pixels are past [`ALPHA_LEVELS`].
    pub alpha: (u8, [u32; 2], usize),
    /// Both as seen, three bytes a pixel, and FLIP's map as a picture.
    pub reference: Vec<u8>,
    pub got: Vec<u8>,
    pub heat: Vec<u8>,
}

/// Measures `got` against `reference`, which are of one size.
pub(crate) fn judge(reference: &Rgba8Image, got: &Rgba8Image) -> Judged {
    let (width, height) = (reference.width(), reference.height());
    let mut alpha = (0, [0, 0], 0);
    let pairs = reference
        .pixels()
        .chunks_exact(4)
        .zip(got.pixels().chunks_exact(4));
    for (at, (a, b)) in pairs.enumerate() {
        let apart = a[3].abs_diff(b[3]);
        if apart > alpha.0 {
            alpha.0 = apart;
            alpha.1 = [at as u32 % width, at as u32 / width];
        }
        alpha.2 += usize::from(apart > ALPHA_LEVELS);
    }
    let (seen_reference, seen_got) = (reference.seen(), got.seen());
    let flip = flip::flip(&seen_reference, &seen_got, width, height);
    let block = colour::worst_block(&seen_reference, &seen_got, width, height);
    Judged {
        measured: Measured {
            flip_mean: flip.mean,
            flip_p99: flip.percentile,
            delta_e: block.delta_e,
        },
        block,
        alpha,
        reference: seen_reference,
        got: seen_got,
        heat: flip.heat,
    }
}

/// The measured numbers against their bounds, as the middle of a
/// sentence.
fn numbers(measured: &Measured, bounds: &Bounds, block: &colour::WorstBlock) -> String {
    format!(
        "FLIP's mean {:.4} of {}, its 99th percentile {:.4} of {}, the worst block's ΔE {:.2} of {} at ({}, {})",
        measured.flip_mean,
        bounds.flip_mean,
        measured.flip_p99,
        bounds.flip_p99,
        measured.delta_e,
        bounds.delta_e,
        block.x,
        block.y
    )
}

impl Golden {
    /// The goldens of the case in `case_dir` (`src/corpus/cases/<case>`),
    /// drawn on `family`. A failure's pictures go in `artefacts`
    /// (`target/corpus/<case>/golden`), which is made when first needed.
    pub fn new(case_dir: &Path, family: Family, artefacts: &Path, mode: Mode) -> Golden {
        Golden {
            case_dir: case_dir.to_path_buf(),
            case: case_dir
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            family,
            artefacts: artefacts.to_path_buf(),
            mode,
        }
    }

    /// Where the reference of the golden `name` of the case in `case_dir`
    /// is kept.
    pub fn reference(case_dir: &Path, name: &str) -> PathBuf {
        case_dir.join(bounds::FOLDER).join(format!("{name}.png"))
    }

    /// Holds `image` to the golden `name`. Never panics and never fails
    /// to answer: whatever goes wrong, down to a reference that is not a
    /// PNG, is a failed result that says what.
    pub fn check(&self, name: &str, image: &Rgba8Image) -> GoldenResult {
        let mut result = GoldenResult {
            case: self.case.clone(),
            name: name.to_owned(),
            family: self.family,
            passed: false,
            unexpected_pass: false,
            blessed: false,
            measured: None,
            worst_block: None,
            bounds: Bounds::DEFAULT,
            bounds_from: Source::Default,
            reference: Self::reference(&self.case_dir, name),
            artefacts: Artefacts::default(),
            detail: String::new(),
        };
        // A name that is not one, or a golden.json that does not hold to
        // its rules, is for a person to put right: blessing does not
        // paper over either.
        let loaded = bounds::check_name(name).and_then(|()| Metadata::load(&self.case_dir));
        let mut metadata = match loaded {
            Ok(metadata) => metadata,
            Err(error) => {
                result.detail = error;
                return result;
            }
        };
        let entry = metadata.entry(name);
        (result.bounds, result.bounds_from) = entry.bounds(self.family);

        // What is wrong, if anything; and the comparison, if there was
        // one to make.
        let mut judged = None;
        let mut old = None;
        let problem = match std::fs::read(&result.reference) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(format!(
                "no reference for {name}; bless it with {BLESS}={}",
                self.case
            )),
            Err(error) => Some(format!(
                "could not read the reference {}: {error}",
                result.reference.display()
            )),
            Ok(bytes) => match Rgba8Image::decode_png(&bytes) {
                Err(error) => Some(format!(
                    "the reference {} does not read: {error}",
                    result.reference.display()
                )),
                Ok(reference) => {
                    let problem = self.compare(&reference, image, &mut result, &mut judged);
                    old = Some(reference);
                    problem
                }
            },
        };

        let forced = self.mode.bless.as_ref().is_some_and(|bless| bless.force);
        if problem.is_none() && !forced {
            result.passed = true;
            let stale = result
                .measured
                .and_then(|measured| entry.stale(self.family, &measured));
            result.unexpected_pass = stale.is_some();
            result.detail = match (stale, &judged) {
                (Some(stale), _) => format!("it passes, unexpectedly: {stale}"),
                (None, Some(judged)) => format!(
                    "{}; {}",
                    numbers(&judged.measured, &result.bounds, &judged.block),
                    result.bounds_from
                ),
                (None, None) => "it passes".to_owned(),
            };
            if result.unexpected_pass {
                // Looked at as a failure is, so the pictures are there.
                self.write_artefacts(&mut result, image, old.as_ref(), judged.as_ref());
            }
            return result;
        }

        self.write_artefacts(&mut result, image, old.as_ref(), judged.as_ref());
        let Some(bless) = &self.mode.bless else {
            result.detail = match &judged {
                Some(_) => format!("{}; {}", problem.unwrap_or_default(), result.bounds_from),
                None => problem.unwrap_or_default(),
            };
            return result;
        };
        let because = problem.unwrap_or_else(|| format!("{BLESS_FORCE} is set"));
        match self.bless(bless, image, &mut metadata, &result, &because) {
            Ok(bytes) => {
                result.passed = true;
                result.blessed = true;
                result.detail = format!(
                    "blessed: {} was written ({bytes} bytes), because {because}",
                    result.reference.display()
                );
                if bytes > budget::REFERENCE_BYTES {
                    result.detail.push_str(&format!(
                        "; it is over the {} bytes a reference may be",
                        budget::REFERENCE_BYTES
                    ));
                }
            }
            Err(error) => result.detail = format!("could not bless {name}: {error}"),
        }
        result
    }

    /// Compares `image` with its reference, filling in what was measured.
    /// What is wrong, if anything.
    fn compare(
        &self,
        reference: &Rgba8Image,
        image: &Rgba8Image,
        result: &mut GoldenResult,
        judged: &mut Option<Judged>,
    ) -> Option<String> {
        if (reference.width(), reference.height()) != (image.width(), image.height()) {
            return Some(format!(
                "the picture is {} × {} and its reference {} × {}",
                image.width(),
                image.height(),
                reference.width(),
                reference.height()
            ));
        }
        let found = judge(reference, image);
        result.measured = Some(found.measured);
        result.worst_block = Some([found.block.x, found.block.y]);
        let mut wrong = Vec::new();
        let (apart, at, pixels) = found.alpha;
        if apart > ALPHA_LEVELS {
            wrong.push(format!(
                "its alpha is not its reference's: {pixels} pixels are more than {ALPHA_LEVELS} level apart, the furthest {apart} levels at ({}, {})",
                at[0], at[1]
            ));
        }
        let within = found.measured.within(&result.bounds);
        let values = found.measured.values();
        for (at, (what, bound)) in result.bounds.named().into_iter().enumerate() {
            if !within[at] {
                wrong.push(format!("{what} is {:.4}, over {bound}", values[at]));
            }
        }
        let problem = (!wrong.is_empty()).then(|| {
            format!(
                "{}; measured {}",
                wrong.join(", and "),
                numbers(&found.measured, &result.bounds, &found.block)
            )
        });
        *judged = Some(found);
        problem
    }

    /// Writes what there is to look at into the artefacts folder: the
    /// picture always, the reference if it read, and the difference and
    /// FLIP's map if the two could be compared. A picture that cannot be
    /// written is left out; the result says what was found either way.
    fn write_artefacts(
        &self,
        result: &mut GoldenResult,
        image: &Rgba8Image,
        reference: Option<&Rgba8Image>,
        judged: Option<&Judged>,
    ) {
        let write = |kind: &str, picture: &Rgba8Image| {
            let path = self.artefacts.join(format!("{}.{kind}.png", result.name));
            picture.write_png(&path).ok().map(|_| path)
        };
        let mut artefacts = Artefacts {
            got: write("got", image),
            reference: reference.and_then(|reference| write("reference", reference)),
            ..Artefacts::default()
        };
        if let Some(judged) = judged {
            let (width, height) = (image.width(), image.height());
            if let Some((picture, gain)) =
                picture::difference(&judged.reference, &judged.got, width, height)
            {
                artefacts.difference = write("difference", &picture);
                artefacts.difference_gain = artefacts.difference.is_some().then_some(gain);
            }
            artefacts.flip = Rgba8Image::from_rgb(width, height, &judged.heat)
                .and_then(|heat| write("flip", &heat));
        }
        result.artefacts = artefacts;
    }

    /// Writes `image` as the reference, notes the family it was drawn on
    /// in `golden.json`, and lists it in the log. How many bytes the
    /// reference is.
    fn bless(
        &self,
        bless: &Bless,
        image: &Rgba8Image,
        metadata: &mut Metadata,
        result: &GoldenResult,
        because: &str,
    ) -> Result<u64, String> {
        let bytes = image.write_png(&result.reference)?;
        metadata
            .goldens
            .entry(result.name.clone())
            .or_default()
            .blessed_on = Some(self.family);
        metadata.save(&self.case_dir)?;

        let shown = |path: &Option<PathBuf>| match path {
            Some(path) => path.display().to_string(),
            None => "none".to_owned(),
        };
        let gain = result
            .artefacts
            .difference_gain
            .map(|gain| format!(" (times {gain})"))
            .unwrap_or_default();
        let entry = format!(
            "\n## {}/{}, drawn on {}\n\n- because: {because}\n- new: {} ({bytes} bytes)\n- old: {}\n- difference: {}{gain}\n- FLIP: {}\n",
            self.case,
            result.name,
            self.family,
            result.reference.display(),
            shown(&result.artefacts.reference),
            shown(&result.artefacts.difference),
            shown(&result.artefacts.flip),
        );
        let fail =
            |error: std::io::Error| format!("could not write {}: {error}", bless.log.display());
        if let Some(folder) = bless.log.parent() {
            std::fs::create_dir_all(folder).map_err(fail)?;
        }
        let new = !bless.log.exists();
        let mut log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&bless.log)
            .map_err(fail)?;
        // One write an entry, so two cases blessing at once do not weave
        // their lines together.
        let heading = "# References blessed\n\nEvery reference a run wrote, with what it replaced. Look at each before committing it.\n";
        let text = if new {
            format!("{heading}{entry}")
        } else {
            entry
        };
        log.write_all(text.as_bytes()).map_err(fail)?;
        Ok(bytes)
    }
}
