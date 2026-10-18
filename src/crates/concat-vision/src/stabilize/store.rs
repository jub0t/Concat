// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Where a media file's measured movement lives between runs.
//!
//! One directory per media file per analysis version, inside the project's
//! `cache/` folder beside the masks, so it travels with the project and a
//! newer analysis never reads an older one's numbers. In it, one small
//! text file, `motion`: the stretches of source measured, and for every
//! frame in them its time and how the camera moved since the frame before.
//! A few dozen bytes a frame; an hour of footage is a few megabytes.
//!
//! The host's analysis writes it whole, by way of a file beside it, so a
//! reader never sees half of one. The renderer reads it through a
//! process-wide cache keyed by the file's length and modification time,
//! since a preview builds its plan again on every edit and the file only
//! changes when an analysis finishes.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use super::ANALYSIS_VERSION;
use super::fit::Similarity;

/// The directory a media file's movement is kept in under `project`.
/// Named like the masks' directories, by a hash of the path, with the
/// analysis's version after it.
pub fn motion_dir(project: &Path, media_path: &str) -> PathBuf {
    project.join("cache").join("stabilize").join(format!(
        "{:016x}-v{ANALYSIS_VERSION}",
        crate::store::fnv1a(media_path.as_bytes())
    ))
}

/// The file in it.
fn motion_file(dir: &Path) -> PathBuf {
    dir.join("motion")
}

/// How a frame's movement was found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    /// The first frame of a stretch: nothing before it was measured, so
    /// nothing is known of how it got here.
    Start,
    /// Measured: the tracks agreed.
    Tracked,
    /// Too little texture to measure. Taken as no movement.
    Flat,
}

impl Step {
    fn letter(self) -> char {
        match self {
            Step::Start => 's',
            Step::Tracked => 't',
            Step::Flat => 'f',
        }
    }

    fn of(letter: &str) -> Option<Step> {
        match letter {
            "s" => Some(Step::Start),
            "t" => Some(Step::Tracked),
            "f" => Some(Step::Flat),
            _ => None,
        }
    }
}

/// One frame's measured movement.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FrameMotion {
    /// When the frame is shown, seconds of source.
    pub at: f64,
    /// How the movement was found.
    pub step: Step,
    /// Where a still point of the scene in the frame before went in this
    /// one, normalised. The identity unless the step is `Tracked`.
    pub motion: Similarity,
}

impl FrameMotion {
    /// The movement to accumulate: the measured one, or none.
    pub fn movement(&self) -> Similarity {
        match self.step {
            Step::Tracked => self.motion,
            Step::Start | Step::Flat => Similarity::IDENTITY,
        }
    }
}

/// A media file's measured movement: what was measured, and every frame
/// of it, in time order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Motion {
    /// The picture's width over its height, as analysed: what the frame's
    /// edges are in normalised units.
    pub aspect: f64,
    /// The stretches of source measured, seconds, in order and apart.
    pub covered: Vec<(f64, f64)>,
    /// The frames, in time order.
    pub frames: Vec<FrameMotion>,
}

/// Two times this close are one frame's.
const SAME_FRAME: f64 = 1e-4;
/// A gap in the coverage narrower than this is no gap: under a frame at
/// the slowest rate anyone shoots.
const SEAM: f64 = 0.05;

impl Motion {
    /// The movement cached in `dir`, or `None` when nothing is, or what is
    /// there cannot be read. Read once per change of the file.
    pub fn read(dir: &Path) -> Option<Arc<Motion>> {
        static CACHE: OnceLock<Mutex<HashMap<PathBuf, (SystemTime, u64, Arc<Motion>)>>> =
            OnceLock::new();
        let file = motion_file(dir);
        let meta = std::fs::metadata(&file).ok()?;
        let stamp = (
            meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            meta.len(),
        );
        let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        if let Ok(cache) = cache.lock()
            && let Some((modified, len, motion)) = cache.get(&file)
            && (*modified, *len) == stamp
        {
            return Some(Arc::clone(motion));
        }
        let text = std::fs::read_to_string(&file).ok()?;
        let motion = Arc::new(Motion::parse(&text)?);
        if let Ok(mut cache) = cache.lock() {
            // A handful of media are stabilized at once; a long session
            // with many is still a few megabytes.
            if cache.len() > 64 {
                cache.clear();
            }
            cache.insert(file, (stamp.0, stamp.1, Arc::clone(&motion)));
        }
        Some(motion)
    }

    /// The text form, read back. `None` for a file of another version or
    /// none at all.
    pub fn parse(text: &str) -> Option<Motion> {
        let mut lines = text.lines();
        let header = lines.next()?;
        if header.trim() != format!("concat-stabilize {ANALYSIS_VERSION}") {
            return None;
        }
        let mut motion = Motion::default();
        for line in lines {
            let mut words = line.split_ascii_whitespace();
            match words.next() {
                Some("aspect") => motion.aspect = words.next()?.parse().ok()?,
                Some("covered") => {
                    let from: f64 = words.next()?.parse().ok()?;
                    let to: f64 = words.next()?.parse().ok()?;
                    motion.covered.push((from, to));
                }
                Some("f") => {
                    let at: f64 = words.next()?.parse().ok()?;
                    let step = Step::of(words.next()?)?;
                    let mut numbers = [0.0f64; 4];
                    for number in &mut numbers {
                        *number = words.next()?.parse().ok()?;
                    }
                    let [a, b, tx, ty] = numbers;
                    motion.frames.push(FrameMotion {
                        at,
                        step,
                        motion: Similarity { a, b, tx, ty },
                    });
                }
                _ => {}
            }
        }
        if !(motion.aspect.is_finite() && motion.aspect > 0.0) {
            return None;
        }
        motion.frames.sort_by(|a, b| a.at.total_cmp(&b.at));
        Some(motion)
    }

    /// The text form.
    pub fn text(&self) -> String {
        let mut out = format!(
            "concat-stabilize {ANALYSIS_VERSION}\naspect {}\n",
            self.aspect
        );
        for (from, to) in &self.covered {
            let _ = writeln!(out, "covered {from:.6} {to:.6}");
        }
        for frame in &self.frames {
            let m = frame.motion;
            let _ = writeln!(
                out,
                "f {:.6} {} {:.9} {:.9} {:.9} {:.9}",
                frame.at,
                frame.step.letter(),
                m.a,
                m.b,
                m.tx,
                m.ty
            );
        }
        out
    }

    /// Writes the movement to `dir`, whole or not at all.
    pub fn write(&self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir)
            .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
        let file = motion_file(dir);
        let partial = file.with_extension("part");
        std::fs::write(&partial, self.text())
            .map_err(|error| format!("could not write {}: {error}", partial.display()))?;
        std::fs::rename(&partial, &file)
            .map_err(|error| format!("could not finish {}: {error}", file.display()))
    }

    /// The parts of `ranges` not measured yet, in order, each a stretch of
    /// source seconds.
    pub fn missing(&self, ranges: &[(f64, f64)]) -> Vec<(f64, f64)> {
        let mut wanted = merged(ranges.to_vec());
        for &(from, to) in &self.covered {
            wanted = wanted
                .into_iter()
                .flat_map(|(a, b)| {
                    if b <= from || a >= to {
                        return vec![(a, b)];
                    }
                    let mut left = Vec::new();
                    if a < from {
                        left.push((a, from));
                    }
                    if b > to {
                        left.push((to, b));
                    }
                    left
                })
                .filter(|(a, b)| b - a > SEAM)
                .collect();
        }
        wanted
    }

    /// Takes in one measured stretch: its frames, and the span of source
    /// it covers. A frame already here is kept, unless it began a stretch
    /// and this one says how it was reached.
    pub fn merge(&mut self, run: &[FrameMotion], span: (f64, f64)) {
        for frame in run {
            let index = self
                .frames
                .partition_point(|f| f.at < frame.at - SAME_FRAME);
            match self.frames.get_mut(index) {
                Some(existing) if (existing.at - frame.at).abs() <= SAME_FRAME => {
                    if existing.step == Step::Start && frame.step != Step::Start {
                        *existing = *frame;
                    }
                }
                _ => self.frames.insert(index, *frame),
            }
        }
        self.covered.push(span);
        self.covered = merged(std::mem::take(&mut self.covered));
    }

    /// The usual time between frames: the median, so a dropped frame or a
    /// seam between stretches does not count. A thirtieth with too few
    /// frames to say.
    pub fn interval(&self) -> f64 {
        let mut gaps: Vec<f64> = self
            .frames
            .windows(2)
            .map(|pair| pair[1].at - pair[0].at)
            .filter(|gap| *gap > SAME_FRAME)
            .collect();
        if gaps.is_empty() {
            return 1.0 / 30.0;
        }
        let middle = gaps.len() / 2;
        let (_, median, _) = gaps.select_nth_unstable_by(middle, f64::total_cmp);
        *median
    }
}

/// Stretches sorted and joined where they meet or overlap.
fn merged(mut ranges: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    ranges.retain(|(a, b)| a.is_finite() && b.is_finite() && b > a);
    ranges.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (a, b) in ranges {
        match out.last_mut() {
            Some(last) if a <= last.1 + SEAM => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(at: f64, step: Step) -> FrameMotion {
        FrameMotion {
            at,
            step,
            motion: Similarity::new(0.001 * at, 1.0, 0.002, -0.001),
        }
    }

    #[test]
    fn the_text_form_reads_back() {
        let mut motion = Motion {
            aspect: 16.0 / 9.0,
            ..Motion::default()
        };
        motion.merge(
            &[
                frame(0.0, Step::Start),
                frame(1.0 / 30.0, Step::Tracked),
                frame(2.0 / 30.0, Step::Flat),
            ],
            (0.0, 0.1),
        );
        let back = Motion::parse(&motion.text()).expect("reads");
        assert_eq!(back.covered, motion.covered);
        assert_eq!(back.frames.len(), 3);
        assert!((back.frames[1].motion.b - motion.frames[1].motion.b).abs() < 1e-9);
        assert_eq!(back.frames[2].step, Step::Flat);
        assert!(Motion::parse("concat-stabilize 0\naspect 1\n").is_none());
    }

    #[test]
    fn what_is_missing_is_what_is_not_covered() {
        let motion = Motion {
            aspect: 1.0,
            covered: vec![(2.0, 5.0), (8.0, 9.0)],
            frames: Vec::new(),
        };
        assert_eq!(
            motion.missing(&[(0.0, 10.0)]),
            vec![(0.0, 2.0), (5.0, 8.0), (9.0, 10.0)]
        );
        assert!(motion.missing(&[(2.5, 4.0), (8.0, 9.02)]).is_empty());
    }

    /// A stretch measured later joins the one before it: the frame that
    /// began the earlier stretch learns how it was reached.
    #[test]
    fn a_later_stretch_joins_an_earlier_one() {
        let mut motion = Motion {
            aspect: 1.0,
            ..Motion::default()
        };
        motion.merge(
            &[frame(1.0, Step::Start), frame(1.1, Step::Tracked)],
            (1.0, 1.1),
        );
        motion.merge(
            &[
                frame(0.8, Step::Start),
                frame(0.9, Step::Tracked),
                frame(1.0, Step::Tracked),
            ],
            (0.8, 1.0),
        );
        let steps: Vec<Step> = motion.frames.iter().map(|f| f.step).collect();
        assert_eq!(
            steps,
            vec![Step::Start, Step::Tracked, Step::Tracked, Step::Tracked]
        );
        assert_eq!(motion.covered, vec![(0.8, 1.1)]);
        assert!((motion.interval() - 0.1).abs() < 1e-9);
    }
}
