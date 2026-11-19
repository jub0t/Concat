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
//! was.
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
}

impl Media {
    /// The media tag the picture's codes carry.
    pub fn tag(&self) -> u8 {
        match self {
            Media::Clock { tag, .. } | Media::Still { tag, .. } | Media::Pips { tag, .. } => *tag,
        }
    }

    /// The file's extension.
    pub fn extension(&self) -> &'static str {
        match self {
            Media::Clock { .. } => "mov",
            Media::Still { .. } => "png",
            Media::Pips { .. } => "wav",
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
            Media::Clock { .. } | Media::Still { .. } => None,
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
    /// Whose pips the sound carries where, as runs over output frames.
    /// Absent, the sound is not checked; present, a pip anywhere they do
    /// not put one is an error, as is silence where they do.
    #[serde(default)]
    pub sound: Vec<SoundRun>,
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
    /// The media they show, by its name in [`Case::media`].
    pub media: String,
    /// The source frame the run's first frame shows.
    pub from: u32,
    /// How many source frames each output frame moves on by: 1 at normal
    /// speed, 0 for a still or a hold. With [`Run::per`], how many source
    /// frames every `per` output frames move on by.
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
}

fn one() -> u32 {
    1
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

    /// The code output frame `frame` must carry and where it sits, by the
    /// runs.
    pub fn expected(&self, frame: u32) -> Option<(code::Code, [u32; 2])> {
        let run = self
            .expect
            .runs
            .iter()
            .find(|run| (run.frames[0]..=run.frames[1]).contains(&frame))?;
        let tag = self.media.get(&run.media)?.tag();
        let index = run.from + (frame - run.frames[0]) * run.step / run.per;
        Some((code::Code { tag, index }, run.at))
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
                let at = start + (second - run.from) / run.speed;
                if at + pip / run.speed > end {
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
                Media::Pips { seconds, .. } => {
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
            if !self.media.contains_key(&run.media) {
                return Err(format!("expect: no media is called {}", run.media));
            }
            if run.per == 0 {
                return Err(format!("expect: the run at {:?} has per 0", run.frames));
            }
            next = run.frames[1] + 1;
        }
        if next != expect.frames {
            return Err(format!(
                "expect: the runs cover {next} frames, and the file has {}",
                expect.frames
            ));
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
            if !run.speed.is_finite() || run.speed <= 0.0 || run.from < 0.0 {
                return Err(format!(
                    "expect: the sound run at {:?} needs a positive speed and a start at or past 0",
                    run.frames
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
                (None, Some(Value::String(name))) if name == "closeAndReopen" => {
                    if object.len() != 1 {
                        return Err(format!("step {at}: closeAndReopen takes nothing"));
                    }
                }
                (None, Some(other)) => {
                    return Err(format!("step {at}: no harness step is called {other}"));
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
