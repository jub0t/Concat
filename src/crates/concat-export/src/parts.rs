// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! An export's picture written in parts, so an export that stops - a
//! driver that fails an hour in, a worker that dies, the app closed - is
//! taken up again at the part it stopped in, not from its first frame.
//!
//! Each part is a file of its own, beside the output in a hidden folder,
//! encoded from a keyframe; a small manifest there says which parts are
//! finished and for which export. The next export of the same edit, at the
//! same size, rate and settings, to the same file, finds them and renders
//! only what is missing; the parts are then copied into one file without
//! being encoded again (`concat_media::join`). A different edit, or other
//! settings, finds a manifest that is not its own and starts afresh.
//!
//! An export short enough that losing it costs little is written whole,
//! as it always was.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use concat_core::frame::YuvLayout;
use concat_core::time::{FrameRate, Rational};
use concat_media::{Encoder, Error};

use crate::{Drawn, Writer};

/// How long a part runs. Long enough that opening an encoder for each
/// costs nothing that shows; short enough that an export that stops loses
/// at most this much of its work.
const PART_SECONDS: i64 = 30;

/// Under this, an export is written whole: a failure costs little, and
/// nothing about its file changes.
const WHOLE_UNDER_SECONDS: i64 = 120;

/// What lets an export be taken up again, from whoever flattened it.
#[derive(Clone, Debug, Default)]
pub struct Resume {
    /// What the edit is: a digest of the project and of the files it
    /// reads. With the same key, size, rate and settings, the parts
    /// already written are the parts this export would write.
    pub key: String,
    /// Frames a part. None: thirty seconds' worth, and an export under two
    /// minutes written whole.
    pub part_frames: Option<i64>,
    /// For tests: after this many parts are written by this run, the
    /// export stops as a crash would, its parts left where they are.
    pub stop_after: Option<usize>,
}

/// The ranges of frames an export of `total` frames is written in.
pub(crate) fn ranges(total: i64, rate: FrameRate, part_frames: Option<i64>) -> Vec<Range<i64>> {
    let fps = rate.fps().as_f64();
    let size = match part_frames {
        Some(frames) => frames.max(1),
        None => {
            if (total as f64) < fps * WHOLE_UNDER_SECONDS as f64 {
                return std::iter::once(0..total).collect();
            }
            ((fps * PART_SECONDS as f64).round() as i64).max(1)
        }
    };
    (0..total)
        .step_by(usize::try_from(size).unwrap_or(usize::MAX))
        .map(|start| start..(start + size).min(total))
        .collect()
}

/// The folder an export to `output` keeps its parts in.
pub(crate) fn folder_for(output: &Path) -> PathBuf {
    let stem = output
        .file_stem()
        .map_or_else(|| "concat".into(), |stem| stem.to_string_lossy());
    output
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!(".{stem}.concat-parts"))
}

/// A stable digest of `text`: two 64-bit SipHashes under different
/// prefixes. Stable within one build, which is all a manifest needs - the
/// build's version is part of what is hashed.
pub fn digest(text: &str) -> String {
    use std::hash::{Hash, Hasher};
    let half = |salt: &str| {
        let mut hasher = std::hash::DefaultHasher::new();
        salt.hash(&mut hasher);
        text.hash(&mut hasher);
        hasher.finish()
    };
    format!(
        "{:016x}{:016x}",
        half("concat-parts-a"),
        half("concat-parts-b")
    )
}

/// Where an export's parts go, and what they are for.
pub(crate) struct Plan {
    pub(crate) folder: PathBuf,
    /// The container the parts are written in, the whole's: an encoder
    /// picks its muxer by the file's extension.
    pub(crate) container: &'static str,
    pub(crate) ranges: Vec<Range<i64>>,
    /// Everything about the export but its encoder's settings, which a pass
    /// may change (see `again_in_software`); [`Split::start`] adds them.
    pub(crate) identity: String,
    pub(crate) stop_after: Option<usize>,
}

/// The manifest's file in a parts folder.
const MANIFEST: &str = "manifest";

/// The parts a folder says are finished for `fingerprint`, in order from
/// the first: only an unbroken run counts, since parts are written in
/// order and one after a gap was not written by this export.
fn finished(folder: &Path, container: &str, fingerprint: &str, count: usize) -> usize {
    let Ok(text) = std::fs::read_to_string(folder.join(MANIFEST)) else {
        return 0;
    };
    let mut lines = text.lines();
    if lines.next() != Some("concat-parts 1") || lines.next() != Some(fingerprint) {
        return 0;
    }
    let mut done = 0;
    for line in lines {
        let Ok(index) = line.trim().parse::<usize>() else {
            break;
        };
        let present = std::fs::metadata(folder.join(part_name(index, container)))
            .is_ok_and(|meta| meta.is_file() && meta.len() > 0);
        if index != done || !present || index >= count {
            break;
        }
        done += 1;
    }
    done
}

fn part_name(index: usize, container: &str) -> String {
    format!("part-{index:05}.{container}")
}

/// Writes the manifest for the first `done` parts, replacing the old one
/// in one rename, so a crash leaves the old manifest or the new one.
fn record(folder: &Path, fingerprint: &str, done: usize) -> std::io::Result<()> {
    let mut text = format!("concat-parts 1\n{fingerprint}\n");
    for index in 0..done {
        text.push_str(&format!("{index}\n"));
    }
    let scratch = folder.join(format!("{MANIFEST}.new"));
    std::fs::write(&scratch, text)?;
    std::fs::rename(scratch, folder.join(MANIFEST))
}

/// Opens an encoder writing to the path given.
pub(crate) type Open = Arc<dyn Fn(&Path) -> Result<Encoder, Error> + Send + Sync>;

/// The picture's writer, whole or in parts. Takes the frames in order and
/// hands each to the part it belongs to, closing one part's encoder and
/// opening the next at each boundary; drops the frames drawn only to warm
/// up an export taken up part way.
pub(crate) struct Split {
    /// Each part's file and its frames. Whole: one part, the destination.
    files: Vec<(PathBuf, Range<i64>)>,
    /// The parts folder, its manifest's fingerprint and the parts in it
    /// that are finished; None when written whole.
    folder: Option<(PathBuf, String)>,
    done: usize,
    /// Where the joined file goes, when written in parts.
    destination: PathBuf,
    rate: FrameRate,
    open: Open,
    /// The part being written, and its encoder.
    current: Option<(usize, Writer)>,
    /// The index of the next frame handed over.
    next: i64,
    /// Frames before this are drawn only to warm the drawing up.
    keep_from: i64,
    /// The YUV the encoder takes, and which encoder it is: the first
    /// part's, which every part shares.
    pub(crate) takes: Option<YuvLayout>,
    pub(crate) encoder: String,
    /// The time the finished parts' encoders spent.
    busy: Duration,
    /// How many frames were written when the encoder stopped, and why.
    stopped: Option<(i64, Error)>,
    stop_after: Option<usize>,
    written_now: usize,
}

impl Split {
    /// The writer for a pass: whole to `destination` when `plan` is None;
    /// otherwise in the plan's parts, taking up those a manifest for this
    /// export says are done. `settings` is what the pass encodes with,
    /// part of the parts' fingerprint. `warm`: frames drawn and dropped
    /// before a part taken up part way, for drawing that remembers the
    /// frames before (a person's cutout).
    pub(crate) fn start(
        destination: &Path,
        total: i64,
        rate: FrameRate,
        plan: Option<&Plan>,
        settings: &str,
        warm: i64,
        open: Open,
    ) -> Result<Split, Error> {
        let mut split = Split {
            files: Vec::new(),
            folder: None,
            done: 0,
            destination: destination.to_path_buf(),
            rate,
            open,
            current: None,
            next: 0,
            keep_from: 0,
            takes: None,
            encoder: String::new(),
            busy: Duration::ZERO,
            stopped: None,
            stop_after: None,
            written_now: 0,
        };
        match plan.filter(|plan| plan.ranges.len() > 1) {
            None => split.files = vec![(destination.to_path_buf(), 0..total)],
            Some(plan) => {
                let fingerprint = digest(&format!("{}\n{settings}", plan.identity));
                let done = finished(
                    &plan.folder,
                    plan.container,
                    &fingerprint,
                    plan.ranges.len(),
                );
                if done == 0 {
                    // Not this export's, or nothing of it kept: started
                    // afresh, so an old export's parts are not left behind.
                    let _ = std::fs::remove_dir_all(&plan.folder);
                    std::fs::create_dir_all(&plan.folder).map_err(|error| Error::Ffi {
                        operation: "make the parts folder",
                        path: plan.folder.clone(),
                        detail: error.to_string(),
                    })?;
                } else {
                    log::info!(
                        "export: taking up where it stopped: {done} of {} parts are written",
                        plan.ranges.len()
                    );
                }
                split.files = plan
                    .ranges
                    .iter()
                    .enumerate()
                    .map(|(index, range)| {
                        (
                            plan.folder.join(part_name(index, plan.container)),
                            range.clone(),
                        )
                    })
                    .collect();
                split.folder = Some((plan.folder.clone(), fingerprint));
                split.done = done;
                split.stop_after = plan.stop_after;
                if let Some((_, range)) = split.files.get(done) {
                    split.keep_from = range.start;
                    split.next = if done == 0 {
                        0
                    } else {
                        (range.start - warm).max(0)
                    };
                } else {
                    split.keep_from = total;
                    split.next = total;
                }
            }
        }
        if split.done < split.files.len() {
            split.open_part(split.done)?;
        }
        Ok(split)
    }

    /// The first frame the pass draws.
    pub(crate) fn first(&self) -> i64 {
        self.next
    }

    /// Whether the pass picks up part way, after parts already written.
    pub(crate) fn taken_up(&self) -> bool {
        self.done > 0
    }

    fn open_part(&mut self, index: usize) -> Result<(), Error> {
        let (path, _) = self.files[index].clone();
        let open = Arc::clone(&self.open);
        let target = path.clone();
        let writer = Writer::start(&path, move || open(&target))?;
        if self.encoder.is_empty() {
            self.takes = writer.takes;
            self.encoder = writer.encoder.clone();
        }
        self.current = Some((index, writer));
        Ok(())
    }

    /// The time the encoders have spent so far.
    pub(crate) fn encoding(&self) -> Duration {
        self.busy
            + self
                .current
                .as_ref()
                .map_or(Duration::ZERO, |(_, writer)| writer.encoding())
    }

    /// Why the export stopped of its own accord: a test's `stop_after`.
    pub(crate) fn halt(&self) -> Option<String> {
        let wanted = self.stop_after?;
        (self.written_now >= wanted).then(|| format!("stopped after {wanted} parts, as asked"))
    }

    /// Hands `drawn` to the part it belongs to. False when the encoder
    /// stopped; [`Split::finish`] says why.
    pub(crate) fn write(&mut self, drawn: Drawn) -> bool {
        if self.stopped.is_some() {
            return false;
        }
        let index = self.next;
        self.next += 1;
        if index < self.keep_from {
            return true;
        }
        let Some((part, writer)) = self.current.as_mut() else {
            // Past the last part: nothing is drawn there.
            return true;
        };
        let part = *part;
        if !writer.write(drawn) {
            let (_, writer) = self.current.take().expect("written to above");
            self.stopped = Some(match writer.finish() {
                Err((written, error)) => (self.files[part].1.start + written, error),
                Ok(_) => unreachable!("an encoder that refused a frame finished"),
            });
            return false;
        }
        if self.next == self.files[part].1.end {
            let (_, writer) = self.current.take().expect("written to above");
            match writer.finish() {
                Ok(busy) => self.busy += busy,
                Err((written, error)) => {
                    self.stopped = Some((self.files[part].1.start + written, error));
                    return false;
                }
            }
            if let Some((folder, fingerprint)) = &self.folder {
                self.done = part + 1;
                self.written_now += 1;
                if let Err(error) = record(folder, fingerprint, self.done) {
                    log::warn!("export: could not note part {part} as written: {error}");
                }
            }
            if part + 1 < self.files.len()
                && self.halt().is_none()
                && let Err(error) = self.open_part(part + 1)
            {
                self.stopped = Some((self.next, error));
                return false;
            }
        }
        true
    }

    /// Finishes the picture once every frame is handed over: the time the
    /// encoders spent, or how many frames were written when one stopped
    /// and why. In parts, the parts are joined into the destination.
    pub(crate) fn finish(mut self) -> Result<Duration, (i64, Error)> {
        if let Some(stopped) = self.stopped.take() {
            return Err(stopped);
        }
        if let Some((_, writer)) = self.current.take() {
            self.busy += writer.finish()?;
        }
        if self.folder.is_none() {
            return Ok(self.busy);
        }
        let parts: Vec<concat_media::join::Part> = self
            .files
            .iter()
            .map(|(path, range)| concat_media::join::Part {
                path: path.clone(),
                start: self.rate.time_of_frame(range.start),
            })
            .collect();
        let total = self.files.last().map_or(0, |(_, range)| range.end);
        if let Err(error) = concat_media::join::join_video(&parts, &self.destination) {
            // Parts that will not join are no use to the next try either.
            if let Some((folder, _)) = &self.folder {
                let _ = std::fs::remove_dir_all(folder);
            }
            return Err((total, error));
        }
        Ok(self.busy)
    }
}

/// How many frames to draw and drop before a part taken up part way, when
/// the drawing remembers the frames before: a second's worth.
pub(crate) fn warm_up(rate: FrameRate, remembers: bool) -> i64 {
    if remembers {
        (rate.fps() * Rational::from_int(1)).floor().max(1)
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_export_is_one_part() {
        let rate = FrameRate::THIRTY;
        assert_eq!(ranges(30 * 119, rate, None), vec![0..30 * 119]);
    }

    #[test]
    fn a_long_export_is_parts_of_thirty_seconds_covering_every_frame_once() {
        let rate = FrameRate::THIRTY;
        let total = 30 * 125 + 7;
        let parts = ranges(total, rate, None);
        assert_eq!(parts.len(), 5);
        assert_eq!(parts[0], 0..900);
        assert_eq!(parts.last().cloned(), Some(3600..total));
        for pair in parts.windows(2) {
            assert_eq!(pair[0].end, pair[1].start, "no gap, no overlap");
        }
    }

    #[test]
    fn a_named_part_size_parts_even_a_short_export() {
        assert_eq!(
            ranges(25, FrameRate::THIRTY, Some(10)),
            vec![0..10, 10..20, 20..25]
        );
    }

    #[test]
    fn only_an_unbroken_run_of_parts_for_this_export_counts() {
        let folder = std::env::temp_dir().join(format!("concat-parts-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("a folder");
        for index in [0, 1, 3] {
            std::fs::write(folder.join(part_name(index, "mp4")), b"x").expect("a part");
        }
        record(&folder, "this", 4).expect("a manifest");
        // Part 2 is missing: parts 0 and 1 are done, 3 is not counted.
        assert_eq!(finished(&folder, "mp4", "this", 5), 2);
        // Another export's manifest counts for nothing.
        assert_eq!(finished(&folder, "mp4", "that", 5), 0);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_digest_is_stable_and_tells_exports_apart() {
        assert_eq!(digest("one"), digest("one"));
        assert_ne!(digest("one"), digest("two"));
        assert_eq!(digest("one").len(), 32);
    }
}
