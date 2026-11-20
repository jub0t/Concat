// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A case as its `case.json` states it: the media to make, the requests
//! that build the project from it, and what the export must then be.
//!
//! The steps are API requests, the JSON a caller on a socket would send,
//! so that when the API's shape changes the scripts change with it, by
//! the same rules, and a case never tests a way into the editor no caller
//! has. Two things are added to a request. `"as"` binds a name to what the
//! request made: on `media.import` the bin entry for that file, on
//! `edit.apply` the id the command minted. And a string that is exactly
//! `"$name"` is replaced by what `name` is bound to before the request is
//! read: `$project` is the project folder, `$file.<media>` the file made
//! for that media, and every `"as"` name the id bound to it (`"$$"` at the
//! start of a string stands for a literal `$`). A step with `"step"` in
//! place of `"method"` is the harness's own: `closeAndReopen` saves and
//! closes the project, opens it again and holds the document to what it
//! was; `removeFile` and `restoreFile` do the same with the named media's
//! file taken away or put back while the project is closed; and
//! `spliceDocument` writes into the saved file, while it is closed, things
//! this build has never heard of, which must be there when it is opened
//! and stay through every save after. `"bind"` on a request binds names to anything else in its answer:
//! each to the string or number a JSON pointer finds in the project the
//! request answered with, `"/project/timelines/0/tracks/0/id"` for the
//! first track's id.
//!
//! The file is read strictly. An unknown field anywhere is an error, as is
//! a missing expectation, so a typo cannot quietly turn a check off.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use crate::code;

/// One case, as its `case.json` has it.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Case {
    /// What the case pins, and what a failure of it means.
    pub about: String,
    /// The project's frame and rate, as `project.create` takes them.
    pub video: Value,
    /// The media to make, by the name the steps call it.
    pub media: BTreeMap<String, Media>,
    /// The requests that build the project, in order.
    pub steps: Vec<Value>,
    /// What the export must be.
    pub expect: Expect,
    /// Projects that must preview the same as this one, pixel for pixel.
    #[serde(default)]
    pub twins: Vec<Twin>,
    /// How the case is exported where not by the sheet's defaults: the
    /// fields of `export.run`'s spec other than its output. What the file
    /// then is goes in [`Expect::file`].
    #[serde(default)]
    pub export: Option<Value>,
}

/// A piece of media to make for a case.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Media {
    /// ProRes 4444 whose every frame carries its own code, silent unless
    /// `sound`.
    Clock {
        /// The media tag its codes carry, 1 to 7.
        tag: u8,
        /// Picture width.
        width: u32,
        /// Picture height.
        height: u32,
        /// Frames a second, as numerator and denominator.
        rate: [i64; 2],
        /// How many frames.
        frames: u32,
        /// A soundtrack of the media's pips (see [`crate::sound`]), as
        /// long as the picture.
        #[serde(default)]
        sound: bool,
    },
    /// A PNG carrying its code as frame nought.
    Still {
        /// The media tag its code carries, 1 to 7.
        tag: u8,
        /// Picture width.
        width: u32,
        /// Picture height.
        height: u32,
    },
    /// A WAV of the media's pips and nothing else: a song.
    Pips {
        /// The media tag its pitch stands for, 1 to 7.
        tag: u8,
        /// How long it is.
        seconds: f64,
    },
    /// A WAV of one steady tone at the media's pitch, for how loud a
    /// sound is where: see [`Expect::level`].
    Tone {
        /// The media tag its pitch stands for, 1 to 7.
        tag: u8,
        /// How long it is.
        seconds: f64,
    },
}

impl Media {
    /// The media tag the picture's codes carry.
    pub fn tag(&self) -> u8 {
        match self {
            Media::Clock { tag, .. }
            | Media::Still { tag, .. }
            | Media::Pips { tag, .. }
            | Media::Tone { tag, .. } => *tag,
        }
    }

    /// The file's extension.
    pub fn extension(&self) -> &'static str {
        match self {
            Media::Clock { .. } => "mov",
            Media::Still { .. } => "png",
            Media::Pips { .. } | Media::Tone { .. } => "wav",
        }
    }

    /// How many seconds of pips it carries, none for a silent one.
    pub fn sound_seconds(&self) -> Option<f64> {
        match self {
            Media::Clock {
                frames,
                rate,
                sound: true,
                ..
            } => Some(f64::from(*frames) * rate[1] as f64 / rate[0] as f64),
            Media::Pips { seconds, .. } => Some(*seconds),
            Media::Clock { .. } | Media::Still { .. } | Media::Tone { .. } => None,
        }
    }
}

/// What the export of a case must be.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Expect {
    /// Frames in the file.
    pub frames: u32,
    /// The file's picture width.
    pub width: u32,
    /// The file's picture height.
    pub height: u32,
    /// The file's frame rate, as numerator and denominator.
    pub rate: [i64; 2],
    /// Which source frame every output frame shows, as runs that between
    /// them cover every frame once, in order.
    pub runs: Vec<Run>,
    /// Rectangles, `[x, y, width, height]`, that are black in every frame:
    /// the bars beside a picture that does not fill the frame.
    #[serde(default)]
    pub black: Vec<[u32; 4]>,
    /// Pictures shown over the runs' own, each smaller than the frame and
    /// clear of the code under it: a picture in a corner. They need not
    /// cover every frame.
    #[serde(default)]
    pub inset: Vec<Run>,
    /// Whose pips the sound carries where, as runs over output frames.
    /// Absent, the sound is not checked; present, a pip anywhere they do
    /// not put one is an error, as is silence where they do.
    #[serde(default)]
    pub sound: Vec<SoundRun>,
    /// How loud a tone is at instants of the export: fades, volume, a
    /// clip that should not be heard at all.
    #[serde(default)]
    pub level: Vec<Level>,
    /// What the exported file is where the export changes it from the
    /// timeline: another size, another rate. Absent, the file is the
    /// timeline's frames at the timeline's size and rate. Everything else
    /// here stays in the timeline's own frames and pixels; a file frame is
    /// held to the timeline frame whose span holds its instant.
    #[serde(default)]
    pub file: Option<File>,
    /// Frames of the monitor held to a stored picture, for what no code
    /// or pip can say: how an effect, a title or a mask looks. See
    /// [`crate::golden`].
    #[serde(default)]
    pub golden: Vec<GoldenFrame>,
}

/// What an exported file is, where it is not the timeline as it stands.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct File {
    /// Frames in the file.
    pub frames: u32,
    /// The file's picture width: the timeline's, times a whole number.
    pub width: u32,
    /// The file's picture height: the timeline's, times the same number.
    pub height: u32,
    /// The file's frame rate, as numerator and denominator.
    pub rate: [i64; 2],
}

/// One frame of the monitor held to a stored picture.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GoldenFrame {
    /// The picture's name in the case's `golden` folder, without `.png`.
    pub name: String,
    /// The output frame, seen at its own instant.
    pub frame: u32,
}

/// How loud one media's tone must be at one instant of the export.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Level {
    /// The instant, in seconds from the start of the file.
    pub at: f64,
    /// The tone, by its name in [`Case::media`].
    pub media: String,
    /// Its amplitude there, of what the file was made at: 1 untouched,
    /// 0.25 half way through a one-second fade, 0 where it is not heard.
    pub gain: f64,
}

/// A step that is the harness's own, not a request.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "step", rename_all = "camelCase", deny_unknown_fields)]
pub enum Harness {
    /// Save, close, reopen; the document must be as it was.
    CloseAndReopen,
    /// The same, with the media's file gone when the project reopens.
    RemoveFile {
        /// The media, by its name in [`Case::media`].
        media: String,
    },
    /// The same, with the media's file back.
    RestoreFile {
        /// The media, by its name in [`Case::media`].
        media: String,
    },
    /// Save and close, write into the saved document, reopen: everything
    /// written must be in the document the project then says it is.
    SpliceDocument {
        /// Values added to the end of arrays in the document.
        #[serde(default)]
        append: Vec<Splice>,
        /// Fields added to objects in the document.
        #[serde(default)]
        merge: Vec<Splice>,
    },
}

/// One thing written into a saved document.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Splice {
    /// A JSON pointer to the array or the object.
    pub at: String,
    /// What is appended to the array, or the fields merged into the object.
    pub value: Value,
}

/// A stretch of the export's sound playing one media's pips.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SoundRun {
    /// The first and last output frame whose span it covers, both
    /// included.
    pub frames: [u32; 2],
    /// The media it plays, by its name in [`Case::media`].
    pub media: String,
    /// The source second playing at the run's first instant.
    pub from: f64,
    /// Source seconds per output second.
    #[serde(default = "unit")]
    pub speed: f64,
    /// A speed curve over the run, in place of `speed`; see [`Run::curve`].
    #[serde(default)]
    pub curve: Option<Vec<[f64; 2]>>,
    /// A flaw this run's sound is known to have: its pips may be off by
    /// up to `within` rather than a millisecond. A run that comes within
    /// the millisecond fails, so the bound is taken out once the flaw is
    /// fixed, and never outlives it.
    #[serde(default)]
    pub known: Option<Known>,
}

/// A known flaw, its measured bound and why it is there.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Known {
    /// How far a pip may be from its place, in seconds.
    pub within: f64,
    /// What causes it, and what fixes it.
    pub why: String,
}

/// Where a pip must be heard in the export, and whose it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pip {
    /// Its onset, in seconds from the start of the file.
    pub at: f64,
    /// The media tag whose pitch it is.
    pub tag: u8,
    /// The source second it starts, for the report.
    pub second: u32,
    /// The sound run it is in, by its place in [`Expect::sound`].
    pub run: usize,
}

/// A run of output frames showing one media's frames at a steady step.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Run {
    /// The first and last output frame of the run, both included.
    pub frames: [u32; 2],
    /// The media they show, by its name in [`Case::media`]. Absent, the
    /// frames show nothing: a gap in the timeline, black from edge to edge.
    #[serde(default)]
    pub media: Option<String>,
    /// The source frame the run's first frame shows.
    #[serde(default)]
    pub from: u32,
    /// How many source frames each output frame moves on by: 1 at normal
    /// speed, 0 for a still or a hold. With [`Run::per`], how many source
    /// frames every `per` output frames move on by.
    #[serde(default)]
    pub step: u32,
    /// The output frames `step` is counted over, 1 unless the speed is a
    /// fraction: 1.5x is step 3 per 2, half speed step 1 per 2. Output
    /// frame `k` of the run shows source frame `from + floor(k * step /
    /// per)`, the frame whose span holds that instant.
    #[serde(default = "one")]
    pub per: u32,
    /// Where in the output frame the picture's code is, `[x, y]`: its top
    /// left corner, the origin unless the picture is placed elsewhere.
    #[serde(default)]
    pub at: [u32; 2],
    /// The side of one block of the code as drawn, 16 unless the picture
    /// is scaled: 8 at half size.
    #[serde(default = "block")]
    pub block: u32,
    /// How much of the picture shows over black at the run's first frame
    /// and at its last, 0 to 1, in light, a straight line between: a clip
    /// whose opacity is set or keyed. Absent, all of it.
    #[serde(default)]
    pub fade: Option<[f64; 2]>,
    /// [`Run::fade`] counted in stops, as an exposure is: at the run's
    /// first frame and at its last, a straight line between, each stop
    /// down halving the light. A keyed exposure is a straight line in
    /// stops and a curve in light.
    #[serde(default)]
    pub stops: Option<[f64; 2]>,
    /// The picture the run's own is melting into, for a run under a
    /// dissolve.
    #[serde(default)]
    pub into: Option<Into>,
    /// A speed curve over the run, in place of `step` and `per`: points
    /// of `[at, speed]`, `at` the fraction of the run and `speed` source
    /// seconds a timeline second there, joined by straight lines, as a
    /// clip's speed curve is. Output frame `k` then shows the source frame
    /// whose span holds the source time the curve has reached.
    #[serde(default)]
    pub curve: Option<Vec<[f64; 2]>>,
}

/// The incoming picture of a run under a dissolve.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Into {
    /// The media coming in, by its name in [`Case::media`].
    pub media: String,
    /// The source frame of it the run's first frame mixes in.
    pub from: u32,
    /// How many of its frames each output frame moves on by; 0 while it is
    /// held on one frame, waiting for its own clip to start.
    pub step: u32,
    /// How much of it is in the run's first frame and in its last, 0 to 1,
    /// in light; in between, a straight line.
    pub share: [f64; 2],
}

/// What an output frame must show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shows {
    /// A source frame, by its code, with the code's top left corner here
    /// and its blocks this many pixels a side.
    Code(code::Code, [u32; 2], u32),
    /// Either of two source frames next to each other: where a speed
    /// curve lands on the very instant one frame gives way to the next,
    /// and which side of it a sum of fractions falls is not the edit's
    /// doing.
    Either(code::Code, code::Code, [u32; 2], u32),
    /// A source frame with this share of it showing over black.
    Faded(code::Code, f64, [u32; 2], u32),
    /// The first source frame melting into the second, with this share of
    /// the second in it.
    Blend(code::Code, code::Code, f64, [u32; 2], u32),
    /// Nothing: black from edge to edge.
    Black,
}

/// A second project, built from the same steps less some, that must
/// preview exactly as the case's own does.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Twin {
    /// What the twin is, for the report: "unsplit".
    pub name: String,
    /// The steps it leaves out, by their `"as"` names.
    pub skip: Vec<String>,
    /// The output frames whose previews are compared, each at its own
    /// instant, `frame / fps`.
    pub frames: Vec<u32>,
    /// Whether the twin's timeline must be the case's own, entry for
    /// entry: an edit and its undoing, a split and a merge. Otherwise it
    /// must differ, or the comparison is of one edit with itself.
    #[serde(default)]
    pub same: bool,
}

/// The speed a curve of `[at, speed]` points gives at fraction `at`:
/// straight lines between the points, the first and last held outside.
fn speed_at(curve: &[[f64; 2]], at: f64) -> f64 {
    let Some(first) = curve.first() else {
        return 1.0;
    };
    if at <= first[0] {
        return first[1];
    }
    for pair in curve.windows(2) {
        let [a, b] = [pair[0], pair[1]];
        if at <= b[0] {
            return a[1] + (b[1] - a[1]) * (at - a[0]) / (b[0] - a[0]).max(f64::EPSILON);
        }
    }
    curve.last().map_or(1.0, |last| last[1])
}

/// The source seconds a run of `length` timeline seconds under `curve`
/// has played by `time` seconds into it: the curve's area up to there.
pub fn source_at(curve: &[[f64; 2]], length: f64, time: f64) -> f64 {
    let reach = (time / length).clamp(0.0, 1.0);
    // The corners up to `reach`, and `reach` itself: between two of them
    // the speed is a straight line, so each stretch is a trapezium.
    let mut corners: Vec<f64> = std::iter::once(0.0)
        .chain(
            curve
                .iter()
                .map(|point| point[0])
                .filter(|at| *at > 0.0 && *at < reach),
        )
        .collect();
    corners.push(reach);
    corners
        .windows(2)
        .map(|pair| {
            (pair[1] - pair[0]) * (speed_at(curve, pair[0]) + speed_at(curve, pair[1])) / 2.0
        })
        .sum::<f64>()
        * length
}

fn one() -> u32 {
    1
}

fn block() -> u32 {
    code::BLOCK
}

fn unit() -> f64 {
    1.0
}

impl Case {
    /// Reads and checks `<cases>/<name>/case.json`. A missing file, one
    /// that does not parse, and one that contradicts itself are errors.
    pub fn load(cases: &Path, name: &str) -> Result<Case, String> {
        let path = cases.join(name).join("case.json");
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        let case: Case = serde_json::from_str(&text)
            .map_err(|error| format!("{} is not a case: {error}", path.display()))?;
        case.check()
            .map_err(|error| format!("{}: {error}", path.display()))?;
        Ok(case)
    }

    /// The output frame rate, in frames a second.
    pub fn fps(&self) -> f64 {
        self.expect.rate[0] as f64 / self.expect.rate[1] as f64
    }

    /// What output frame `frame` must show, by the runs.
    pub fn expected(&self, frame: u32) -> Option<Shows> {
        let run = self
            .expect
            .runs
            .iter()
            .find(|run| (run.frames[0]..=run.frames[1]).contains(&frame))?;
        self.shows(run, frame)
    }

    /// Every picture output frame `frame` must show over the one the runs
    /// give it: the insets.
    pub fn expected_insets(&self, frame: u32) -> Vec<Shows> {
        self.expect
            .inset
            .iter()
            .filter(|run| (run.frames[0]..=run.frames[1]).contains(&frame))
            .filter_map(|run| self.shows(run, frame))
            .collect()
    }

    fn shows(&self, run: &Run, frame: u32) -> Option<Shows> {
        let Some(media) = &run.media else {
            return Some(Shows::Black);
        };
        let spec = self.media.get(media)?;
        let tag = spec.tag();
        let Some(curve) = &run.curve else {
            let along = frame - run.frames[0];
            let index = run.from + along * run.step / run.per;
            let code = code::Code { tag, index };
            let span = f64::from((run.frames[1] - run.frames[0]).max(1));
            if let Some(fade) = run.fade {
                let share = fade[0] + (fade[1] - fade[0]) * f64::from(along) / span;
                return Some(Shows::Faded(code, share, run.at, run.block));
            }
            if let Some(stops) = run.stops {
                let at = stops[0] + (stops[1] - stops[0]) * f64::from(along) / span;
                return Some(Shows::Faded(code, 2f64.powf(at), run.at, run.block));
            }
            let Some(into) = &run.into else {
                return Some(Shows::Code(code, run.at, run.block));
            };
            let coming = code::Code {
                tag: self.media.get(&into.media)?.tag(),
                index: into.from + along * into.step,
            };
            let share = into.share[0] + (into.share[1] - into.share[0]) * f64::from(along) / span;
            return Some(Shows::Blend(code, coming, share, run.at, run.block));
        };
        let Media::Clock { rate, .. } = spec else {
            return None;
        };
        let length = f64::from(run.frames[1] - run.frames[0] + 1) / self.fps();
        let time = f64::from(frame - run.frames[0]) / self.fps();
        let source = source_at(curve, length, time) * rate[0] as f64 / rate[1] as f64;
        let index = run.from + source.floor() as u32;
        let code = code::Code { tag, index };
        // Within a millionth of a frame of a boundary, either side is right.
        let near = source - source.round();
        Some(if near.abs() < 1e-6 && source.round() >= 1.0 {
            let other = run.from + source.round() as u32 - u32::from(near >= 0.0);
            let other = if other == index { index + 1 } else { other };
            Shows::Either(code, code::Code { tag, index: other }, run.at, run.block)
        } else {
            Shows::Code(code, run.at, run.block)
        })
    }

    /// The timeline frame whose span holds file frame `frame`'s instant,
    /// and how many times larger than the timeline the file's picture is.
    pub fn file_frame(&self, frame: u32) -> (u32, u32) {
        let Some(file) = &self.expect.file else {
            return (frame, 1);
        };
        let rate = self.expect.rate;
        let at = i128::from(frame) * i128::from(file.rate[1]) * i128::from(rate[0])
            / (i128::from(file.rate[0]) * i128::from(rate[1]));
        (at as u32, file.width / self.expect.width)
    }

    /// Every pip the export's sound must carry, in order of onset: each
    /// whole pip a sound run's stretch holds, of the source seconds its
    /// media has room for.
    pub fn expected_pips(&self) -> Vec<Pip> {
        let fps = self.fps();
        let pip = crate::sound::PIP_SECONDS;
        let mut pips = Vec::new();
        for (index, run) in self.expect.sound.iter().enumerate() {
            let Some(media) = self.media.get(&run.media) else {
                continue;
            };
            let Some(length) = media.sound_seconds() else {
                continue;
            };
            let start = f64::from(run.frames[0]) / fps;
            let end = f64::from(run.frames[1] + 1) / fps;
            let mut second = run.from.ceil();
            while second + pip <= length {
                // Where the run reaches that source second, and how fast
                // it is going there.
                let (at, speed) = match &run.curve {
                    None => (start + (second - run.from) / run.speed, run.speed),
                    Some(curve) => {
                        let span = end - start;
                        if source_at(curve, span, span) < second - run.from {
                            break;
                        }
                        let (mut low, mut high) = (0.0, span);
                        for _ in 0..60 {
                            let middle = (low + high) / 2.0;
                            if source_at(curve, span, middle) < second - run.from {
                                low = middle;
                            } else {
                                high = middle;
                            }
                        }
                        (start + high, speed_at(curve, high / span))
                    }
                };
                if at + pip / speed > end {
                    break;
                }
                pips.push(Pip {
                    at,
                    tag: media.tag(),
                    second: second as u32,
                    run: index,
                });
                second += 1.0;
            }
        }
        pips.sort_by(|a, b| a.at.total_cmp(&b.at));
        pips
    }

    /// The names the steps bind with `"as"`.
    pub fn bound_names(&self) -> BTreeSet<String> {
        self.steps
            .iter()
            .filter_map(|step| step.get("as").and_then(Value::as_str))
            .map(str::to_owned)
            .collect()
    }

    /// Whatever in the case cannot be right, before anything is run.
    fn check(&self) -> Result<(), String> {
        let mut tags = BTreeSet::new();
        for (name, media) in &self.media {
            let tag = media.tag();
            if !(1..=code::MAX_TAG).contains(&tag) {
                return Err(format!("media {name}: a tag is 1 to {}", code::MAX_TAG));
            }
            if !tags.insert(tag) {
                return Err(format!("media {name}: tag {tag} is another media's too"));
            }
            let (width, height) = match media {
                Media::Clock {
                    width,
                    height,
                    frames,
                    rate,
                    ..
                } => {
                    if *frames == 0 || *frames > code::MAX_INDEX + 1 {
                        return Err(format!("media {name}: 1 to {} frames", code::MAX_INDEX + 1));
                    }
                    if rate[0] <= 0 || rate[1] <= 0 {
                        return Err(format!("media {name}: the rate must be positive"));
                    }
                    (*width, *height)
                }
                Media::Still { width, height, .. } => (*width, *height),
                Media::Pips { seconds, .. } | Media::Tone { seconds, .. } => {
                    if !seconds.is_finite() || *seconds <= 0.0 {
                        return Err(format!("media {name}: a song lasts more than nothing"));
                    }
                    continue;
                }
            };
            if width < code::WIDTH || height < code::HEIGHT || width % 2 == 1 || height % 2 == 1 {
                return Err(format!(
                    "media {name}: {width}×{height} must be even and at least {}×{} for its code",
                    code::WIDTH,
                    code::HEIGHT
                ));
            }
        }
        let expect = &self.expect;
        if expect.rate[0] <= 0 || expect.rate[1] <= 0 {
            return Err("expect: the rate must be positive".to_owned());
        }
        // Every output frame is in exactly one run, in order, so no frame
        // goes unchecked.
        let mut next = 0;
        for run in &expect.runs {
            if run.frames[0] != next || run.frames[1] < run.frames[0] {
                return Err(format!(
                    "expect: the runs must cover every frame from 0 in order; a run starting at {} was expected, not {:?}",
                    next, run.frames
                ));
            }
            match &run.media {
                Some(media) if !self.media.contains_key(media) => {
                    return Err(format!("expect: no media is called {media}"));
                }
                None if run.from != 0 || run.step != 0 => {
                    return Err(format!(
                        "expect: the run at {:?} shows no media, so has no from or step",
                        run.frames
                    ));
                }
                _ => {}
            }
            if run.per == 0 {
                return Err(format!("expect: the run at {:?} has per 0", run.frames));
            }
            if let Some(curve) = &run.curve {
                check_curve(curve)
                    .map_err(|error| format!("expect: the run at {:?}: {error}", run.frames))?;
            }
            if let Some(into) = &run.into {
                let shares = into.share.iter().all(|share| (0.0..=1.0).contains(share));
                if !self.media.contains_key(&into.media) || !shares || run.media.is_none() {
                    return Err(format!(
                        "expect: the run at {:?} melts a media of the case into another, by shares of 0 to 1",
                        run.frames
                    ));
                }
            }
            next = run.frames[1] + 1;
        }
        if next != expect.frames {
            return Err(format!(
                "expect: the runs cover {next} frames, and the file has {}",
                expect.frames
            ));
        }
        for run in &expect.inset {
            let known = run
                .media
                .as_ref()
                .is_some_and(|media| self.media.contains_key(media));
            if !known
                || run.per == 0
                || run.frames[1] < run.frames[0]
                || run.frames[1] >= expect.frames
            {
                return Err(format!(
                    "expect: the inset at {:?} needs a media of the case, frames inside the file and a per over 0",
                    run.frames
                ));
            }
        }
        for run in &expect.sound {
            let Some(media) = self.media.get(&run.media) else {
                return Err(format!("expect: no media is called {}", run.media));
            };
            if media.sound_seconds().is_none() {
                return Err(format!("expect: {} has no sound to hear", run.media));
            }
            if run.frames[1] < run.frames[0] || run.frames[1] >= expect.frames {
                return Err(format!(
                    "expect: the sound run at {:?} is not inside the file's frames",
                    run.frames
                ));
            }
            if let Some(known) = &run.known
                && !(known.within > 0.0 && known.within < 0.25)
            {
                return Err(format!(
                    "expect: the sound run at {:?} is known off by {} s; a bound is over 0 and under 0.25 s",
                    run.frames, known.within
                ));
            }
            if let Some(curve) = &run.curve {
                check_curve(curve).map_err(|error| {
                    format!("expect: the sound run at {:?}: {error}", run.frames)
                })?;
            }
            if !run.speed.is_finite() || run.speed <= 0.0 || run.from < 0.0 {
                return Err(format!(
                    "expect: the sound run at {:?} needs a positive speed and a start at or past 0",
                    run.frames
                ));
            }
        }
        if let Some(file) = &expect.file {
            let times = file.width / expect.width.max(1);
            if times == 0
                || file.width != expect.width * times
                || file.height != expect.height * times
                || file.rate[0] <= 0
                || file.rate[1] <= 0
            {
                return Err(
                    "expect: the file is the timeline's size times a whole number, at a positive rate"
                        .to_owned(),
                );
            }
        }
        for golden in &expect.golden {
            crate::golden::bounds::check_name(&golden.name)
                .map_err(|error| format!("expect: golden {}: {error}", golden.name))?;
            if golden.frame >= expect.frames {
                return Err(format!(
                    "expect: golden {} is of frame {}, past the end",
                    golden.name, golden.frame
                ));
            }
        }
        for level in &expect.level {
            if !matches!(self.media.get(&level.media), Some(Media::Tone { .. })) {
                return Err(format!("expect: {} is no tone to measure", level.media));
            }
            if !(level.at >= 0.0 && level.gain >= 0.0 && level.gain.is_finite()) {
                return Err(format!(
                    "expect: the level of {} at {} s needs an instant and a gain at or past 0",
                    level.media, level.at
                ));
            }
        }
        for rect in &expect.black {
            if rect[0] + rect[2] > expect.width || rect[1] + rect[3] > expect.height {
                return Err(format!(
                    "expect: the black rectangle {rect:?} leaves the frame"
                ));
            }
        }
        let bound = self.bound_names();
        for twin in &self.twins {
            for name in &twin.skip {
                if !bound.contains(name) {
                    return Err(format!(
                        "twin {}: no step is bound as {name} to leave out",
                        twin.name
                    ));
                }
            }
            if twin.skip.is_empty() || twin.frames.is_empty() {
                return Err(format!(
                    "twin {}: a twin leaves a step out and compares frames",
                    twin.name
                ));
            }
            if let Some(frame) = twin.frames.iter().find(|frame| **frame >= expect.frames) {
                return Err(format!("twin {}: frame {frame} is past the end", twin.name));
            }
        }
        for (at, step) in self.steps.iter().enumerate() {
            let object = step
                .as_object()
                .ok_or_else(|| format!("step {at} is not an object"))?;
            match (object.get("method"), object.get("step")) {
                (Some(_), None) => {}
                (None, Some(_)) => {
                    let harness: Harness = serde_json::from_value(step.clone())
                        .map_err(|error| format!("step {at} is no harness step: {error}"))?;
                    if let Harness::RemoveFile { media } | Harness::RestoreFile { media } = &harness
                        && !self.media.contains_key(media)
                    {
                        return Err(format!("step {at}: no media is called {media}"));
                    }
                }
                _ => {
                    return Err(format!(
                        "step {at}: a step has a method or a step, not both"
                    ));
                }
            }
        }
        Ok(())
    }
}

/// A curve has points in order from 0 to 1, each with a speed over 0.
fn check_curve(curve: &[[f64; 2]]) -> Result<(), String> {
    let ordered = curve.windows(2).all(|pair| pair[0][0] < pair[1][0]);
    let inside = curve
        .iter()
        .all(|point| (0.0..=1.0).contains(&point[0]) && point[1] > 0.0 && point[1].is_finite());
    if curve.len() < 2 || !ordered || !inside {
        return Err(
            "a curve is two or more [at, speed] points, at rising from 0 to 1, speed over 0"
                .to_owned(),
        );
    }
    Ok(())
}

/// Every case directory under `cases`, by name.
pub fn names(cases: &Path) -> Result<Vec<String>, String> {
    let entries = std::fs::read_dir(cases)
        .map_err(|error| format!("could not list {}: {error}", cases.display()))?;
    let mut names = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("could not list {}: {error}", cases.display()))?;
        if entry.path().is_dir() {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    Ok(names)
}
