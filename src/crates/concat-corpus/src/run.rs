// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Running one case: make its media, build the project through the API,
//! save, close and reopen it, export it the way the window does, read the
//! file back and check every frame of it.
//!
//! Everything that can be checked is, and a failed check does not stop the
//! run: a red case lists all that is wrong with it, not the first thing.
//! What does stop it is anything that leaves nothing to check - a request
//! the API refuses, an export that fails - and that is reported as the
//! run's error.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use base64::Engine as _;
use concat_api::{Api, AppDirs, Event, ExportSpec, Reply, Request, Response};
use concat_core::frame::Frame;
use concat_core::time::{FrameRate, Rational};
use concat_media::{DecodeOptions, Decoder, FrameSource};
use serde_json::{Value, json};

use crate::case::{Case, Media};
use crate::code;
use crate::media;
use crate::report::Report;

/// The longest an export may take before the run calls it stuck: far past
/// the seconds a few seconds of 320 × 180 take, even on a software adapter.
const EXPORT_TIMEOUT: Duration = Duration::from_secs(300);

/// The most frames one run writes out for a report: enough to see what
/// went wrong, few enough that a case wrong in every frame does not fill
/// the disk.
const MAX_SAVED: usize = 24;

/// The most failures one check names in its detail; the rest are counted.
const MAX_NAMED: usize = 6;

/// The brightest a channel of a bar may be and still be black, after an
/// encoder has had it: the picture beside a bar is at 96 and over in every
/// channel but blue (see [`media::picture`]), so a picture where a bar
/// should be is far past this.
const BLACK: u8 = 40;

/// Runs case `name` from `cases`, writing its report and any failed frames
/// to `out/<name>/`. That folder is emptied first. The working files - the
/// media, the projects, the export - are kept under `work/` there when the
/// case fails and removed when it passes.
pub fn run_case(name: &str, cases: &Path, out: &Path) -> Report {
    let started = Instant::now();
    let dir = out.join(name);
    let mut report = Report {
        case: name.to_owned(),
        ..Report::default()
    };
    if let Err(error) = run(name, cases, &dir, &mut report) {
        report.error = Some(error);
    }
    report.settle();
    report.seconds = started.elapsed().as_secs_f64();
    if report.passed {
        let _ = std::fs::remove_dir_all(dir.join("work"));
        report.export = None;
    }
    if let Err(error) = report.write(&dir) {
        report.error.get_or_insert(error);
        report.passed = false;
    }
    report
}

fn run(name: &str, cases: &Path, dir: &Path, report: &mut Report) -> Result<(), String> {
    let case = Case::load(cases, name)?;
    if dir.exists() {
        std::fs::remove_dir_all(dir)
            .map_err(|error| format!("could not empty {}: {error}", dir.display()))?;
    }
    let work = dir.join("work");
    for folder in [dir.join("frames"), work.join("media"), work.join("app")] {
        std::fs::create_dir_all(&folder)
            .map_err(|error| format!("could not make {}: {error}", folder.display()))?;
    }
    // Absolute, as the paths a document holds are. Not canonical: on
    // Windows that is the `\\?\` form, which is not what a person's paths
    // look like, so not what the app is tested with. Where a path the API
    // hands back is compared with one made here, both are made canonical
    // for the comparison alone.
    let work = std::path::absolute(&work)
        .map_err(|error| format!("could not resolve {}: {error}", work.display()))?;

    let mut files = BTreeMap::new();
    for (media_name, spec) in &case.media {
        let path = work
            .join("media")
            .join(format!("{media_name}.{}", spec.extension()));
        make(&path, spec)?;
        files.insert(media_name.clone(), path);
    }

    let (sender, events) = mpsc::channel();
    let mut api = Api::with_dirs(
        AppDirs::under(&work.join("app")),
        Arc::new(move |event| {
            let _ = sender.send(event);
        }),
    );
    api.restrict_writes_to(vec![work.clone()]);
    let mut runner = Runner {
        case: &case,
        name,
        dir: dir.to_path_buf(),
        work,
        api,
        events,
        files,
        saved: 0,
    };

    let project = runner.build(report, "case", &[])?;
    let output = runner.export(&project)?;
    report.export = Some(output.display().to_string());
    runner.check_export(report, &project, &output)?;
    for twin in &case.twins {
        runner.check_twin(report, &project, twin)?;
    }
    runner.check_leftovers(report);
    Ok(())
}

/// Makes one piece of media.
fn make(path: &Path, spec: &Media) -> Result<(), String> {
    match spec {
        Media::Clock {
            tag,
            width,
            height,
            rate,
            frames,
        } => media::clock(
            path,
            *width,
            *height,
            FrameRate::new(Rational::new(rate[0], rate[1])),
            *frames,
            *tag,
        ),
        Media::Still { tag, width, height } => media::still(path, *width, *height, *tag),
    }
}

struct Runner<'a> {
    case: &'a Case,
    name: &'a str,
    /// `out/<case>`: the report and the frames written for it.
    dir: PathBuf,
    /// Where the media, the app's folders, the projects and the export go,
    /// and the one place the API may write.
    work: PathBuf,
    api: Api,
    events: Receiver<Event>,
    /// The file made for each media, by name.
    files: BTreeMap<String, PathBuf>,
    /// Frames written out so far.
    saved: usize,
}

impl Runner<'_> {
    /// Sends one request; a refusal is the run's error.
    fn send(&mut self, request: Request) -> Result<Reply, String> {
        let method = serde_json::to_value(&request)
            .ok()
            .and_then(|value| {
                value
                    .get("method")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        match self.api.dispatch(request) {
            Response::Result(reply) => Ok(reply),
            Response::Error(error) => Err(format!(
                "{method} was refused ({:?}): {}",
                error.code, error.message
            )),
        }
    }

    /// The project's document, as a save would write it.
    fn document(&mut self, project: &str) -> Result<Value, String> {
        match self.send(Request::ProjectDocument {
            path: project.to_owned(),
        })? {
            Reply::Document(document) => Ok(document),
            _ => Err("project.document did not answer with a document".to_owned()),
        }
    }

    /// Builds the case's project in `work/<label>`, leaving out the steps
    /// bound as any of `skip`, then saves, closes and reopens it. Its folder.
    fn build(
        &mut self,
        report: &mut Report,
        label: &str,
        skip: &[String],
    ) -> Result<String, String> {
        let location = self.work.join(label);
        std::fs::create_dir_all(&location)
            .map_err(|error| format!("could not make {}: {error}", location.display()))?;
        let create: Request = serde_json::from_value(json!({
            "method": "project.create",
            "location": location.to_string_lossy(),
            "name": self.name,
            "video": self.case.video,
        }))
        .map_err(|error| format!("the case's video is not one project.create takes: {error}"))?;
        self.send(create)?;
        let project = self
            .api
            .recents()
            .into_iter()
            .map(|info| info.path)
            .find(|path| canonical(path).starts_with(canonical(&location.to_string_lossy())))
            .ok_or_else(|| {
                format!(
                    "the project made in {} is not in the recents",
                    location.display()
                )
            })?;

        let mut bindings = BTreeMap::new();
        bindings.insert("project".to_owned(), project.clone());
        for (name, path) in &self.files {
            bindings.insert(format!("file.{name}"), path.to_string_lossy().into_owned());
        }
        for (at, step) in self.case.steps.iter().enumerate() {
            let alias = step.get("as").and_then(Value::as_str);
            if alias.is_some_and(|alias| skip.iter().any(|name| name == alias)) {
                continue;
            }
            if step.get("step").is_some() {
                self.round_trip(report, label, &project, &format!("step {at}"))?;
                continue;
            }
            self.script_step(at, step, &mut bindings)?;
        }
        self.round_trip(report, label, &project, "the end")?;
        Ok(project)
    }

    /// Runs one request of the script, binding what it made when it says
    /// `"as"`.
    fn script_step(
        &mut self,
        at: usize,
        step: &Value,
        bindings: &mut BTreeMap<String, String>,
    ) -> Result<(), String> {
        let mut value =
            substitute(step, bindings).map_err(|error| format!("step {at}: {error}"))?;
        let alias = value
            .as_object_mut()
            .and_then(|object| object.remove("as"))
            .map(|alias| {
                alias
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| format!("step {at}: \"as\" is a name"))
            })
            .transpose()?;
        let request: Request = serde_json::from_value(value.clone())
            .map_err(|error| format!("step {at} is not a request: {error}"))?;
        // A field the API does not read is dropped by serde without a word;
        // here it is an error, so a misspelt field is not a silent default.
        let read = serde_json::to_value(&request)
            .map_err(|error| format!("step {at}: the request does not encode: {error}"))?;
        covered(&value, &read, "")
            .map_err(|path| format!("step {at}: {path} is not a field the request reads"))?;
        let method = value["method"].as_str().unwrap_or_default().to_owned();
        let reply = self
            .send(request)
            .map_err(|error| format!("step {at}: {error}"))?;
        let Some(alias) = alias else {
            return Ok(());
        };
        let Reply::View(view) = reply else {
            return Err(format!(
                "step {at}: {method} answered with no project to bind from"
            ));
        };
        let id = match method.as_str() {
            "media.import" => {
                let file = value["file"].as_str().unwrap_or_default();
                let wanted = canonical(file);
                view.project
                    .media
                    .iter()
                    .find(|item| canonical(&item.path) == wanted)
                    .map(|item| item.id.clone())
                    .ok_or_else(|| {
                        format!("step {at}: {file} is not in the bin after its import")
                    })?
            }
            "edit.apply" => view
                .created_id
                .clone()
                .ok_or_else(|| format!("step {at}: the edit minted nothing to bind as {alias}"))?,
            _ => {
                return Err(format!(
                    "step {at}: only media.import and edit.apply bind a name, not {method}"
                ));
            }
        };
        if bindings.insert(alias.clone(), id).is_some() {
            return Err(format!("step {at}: {alias} is bound already"));
        }
        Ok(())
    }

    /// Saves, closes and reopens the project and checks the document came
    /// back as it went.
    fn round_trip(
        &mut self,
        report: &mut Report,
        label: &str,
        project: &str,
        when: &str,
    ) -> Result<(), String> {
        let before = self.document(project)?;
        self.send(Request::ProjectClose {
            path: project.to_owned(),
            save: true,
        })?;
        self.send(Request::ProjectOpen {
            path: project.to_owned(),
        })?;
        let after = self.document(project)?;
        let detail = match difference(&before, &after, "") {
            None => "equal".to_owned(),
            Some(path) => {
                let stem = format!("{label}-{}", when.replace(' ', "-"));
                for (side, document) in [("before", &before), ("after", &after)] {
                    let file = self.dir.join(format!("{stem}-document-{side}.json"));
                    let _ = std::fs::write(
                        &file,
                        serde_json::to_string_pretty(document).unwrap_or_default(),
                    );
                }
                format!("they first differ at {path}; both are beside this report")
            }
        };
        report.check(
            format!("{label}: the document is the same after it is saved, closed and reopened, at {when}"),
            before == after,
            detail,
        );
        Ok(())
    }

    /// Exports the project with the sheet's defaults into its own folder,
    /// and waits for the file.
    fn export(&mut self, project: &str) -> Result<PathBuf, String> {
        let output = Path::new(project).join(format!("{}.mp4", self.name));
        let reply = self.send(Request::ExportRun {
            path: project.to_owned(),
            spec: ExportSpec {
                output: output.to_string_lossy().into_owned(),
                ..ExportSpec::default()
            },
        })?;
        let Reply::Started(started) = reply else {
            return Err("export.run did not start a job".to_owned());
        };
        let deadline = Instant::now() + EXPORT_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.events.recv_timeout(left) {
                Ok(Event::ExportDone { job, output, .. }) if job == started.job => {
                    return Ok(PathBuf::from(output));
                }
                Ok(Event::ExportFailed { job, error, .. }) if job == started.job => {
                    return Err(format!("the export failed: {}", error.message));
                }
                Ok(_) => {}
                Err(_) => {
                    return Err(format!(
                        "the export did not finish in {} s",
                        EXPORT_TIMEOUT.as_secs()
                    ));
                }
            }
        }
    }

    /// Everything the exported file must be: its size, rate and length,
    /// and the source frame in every one of its frames.
    fn check_export(
        &mut self,
        report: &mut Report,
        project: &str,
        output: &Path,
    ) -> Result<(), String> {
        let expect = &self.case.expect;
        let info = concat_media::probe(output)
            .map_err(|error| format!("the export does not probe: {error}"))?;
        let video = info
            .video
            .as_ref()
            .ok_or_else(|| "the export has no picture".to_owned())?;
        let rate = video.frame_rate.fps();
        let wanted = Rational::new(expect.rate[0], expect.rate[1]);
        report.check(
            format!(
                "the file is {}×{} at {}/{} fps",
                expect.width, expect.height, expect.rate[0], expect.rate[1]
            ),
            video.width == expect.width && video.height == expect.height && rate == wanted,
            format!(
                "it is {}×{} at {}/{} fps",
                video.width,
                video.height,
                rate.numerator(),
                rate.denominator()
            ),
        );

        let end = match self.send(Request::ProjectGet {
            path: project.to_owned(),
        })? {
            Reply::View(view) => view
                .project
                .active()
                .clips
                .iter()
                .map(|clip| clip.start + clip.duration)
                .fold(0.0, f64::max),
            _ => return Err("project.get did not answer with the project".to_owned()),
        };
        let length = (end * self.case.fps()).round() as u32;
        report.check(
            "the timeline's length at its rate is the frame count the case expects",
            length == expect.frames,
            format!(
                "the timeline ends at {end} s, {length} frames; the case expects {}",
                expect.frames
            ),
        );

        let frames = decode(output)?;
        report.check(
            format!("the file holds {} frames", expect.frames),
            frames.len() == expect.frames as usize,
            format!("it holds {}", frames.len()),
        );

        let mut wrong = Vec::new();
        let mut worst: f64 = 0.0;
        for index in 0..expect.frames {
            let (code, at) = self
                .case
                .expected(index)
                .ok_or_else(|| format!("no run covers frame {index}"))?;
            let Some(frame) = frames.get(index as usize) else {
                wrong.push(format!("frame {index} is missing"));
                continue;
            };
            let reading = code::read_at(frame, at[0], at[1]);
            worst = worst.max(reading.worst);
            if reading.code != Some(code) {
                wrong.push(format!(
                    "frame {index} should be {code}, and reads as {}",
                    describe(&reading)
                ));
                self.save(report, &format!("export-{index:04}.png"), frame);
            }
        }
        report.code_worst = worst;
        report.check(
            "every frame shows the source frame the case expects",
            wrong.is_empty(),
            named(
                &wrong,
                &format!("every code read, the worst block {worst:.1} levels off its level"),
            ),
        );

        if !expect.black.is_empty() {
            let mut grey = Vec::new();
            for (index, frame) in frames.iter().enumerate() {
                for rect in &expect.black {
                    if let Some(brightest) = brightest(frame, rect)
                        && brightest > BLACK
                    {
                        grey.push(format!("frame {index}: {rect:?} reaches {brightest}"));
                        self.save(report, &format!("export-{index:04}.png"), frame);
                    }
                }
            }
            report.check(
                "the bars are black in every frame",
                grey.is_empty(),
                named(&grey, "black"),
            );
        }
        Ok(())
    }

    /// Builds `twin` and holds its previews to the case's own, byte for
    /// byte, at each of its frames; and holds the case's previews to the
    /// codes the export should show there.
    fn check_twin(
        &mut self,
        report: &mut Report,
        project: &str,
        twin: &crate::case::Twin,
    ) -> Result<(), String> {
        let label = format!("twin-{}", twin.name);
        let other = self.build(report, &label, &twin.skip)?;
        let ours = self.document(project)?;
        let theirs = self.document(&other)?;
        let differ = difference(&ours["timelines"], &theirs["timelines"], "").is_some();
        report.check(
            format!("the {} twin's timeline differs from the case's", twin.name),
            differ,
            if differ {
                "it does, so the comparison is of two different edits".to_owned()
            } else {
                "they are the same edit, which would make the comparison prove nothing".to_owned()
            },
        );

        let fps = self.case.fps();
        let mut unequal = Vec::new();
        let mut wrong = Vec::new();
        for &index in &twin.frames {
            // The frame's own instant, not the middle of its span: the
            // monitor shows the frame nearest the instant it is asked for
            // (`quantise` in concat-export's resolve.rs), so the middle of
            // a span is exactly where it is a coin toss between two
            // frames, and the instant itself is the middle of the half a
            // frame either side that shows it.
            let time = f64::from(index) / fps;
            let mine = self.preview(project, time)?;
            let yours = self.preview(&other, time)?;
            if mine != yours {
                let pixels = mine
                    .pixels()
                    .chunks_exact(4)
                    .zip(yours.pixels().chunks_exact(4))
                    .filter(|(a, b)| a != b)
                    .count();
                unequal.push(format!("frame {index}: {pixels} pixels differ"));
                self.save(report, &format!("preview-{index:04}-case.png"), &mine);
                self.save(
                    report,
                    &format!("preview-{index:04}-{}.png", twin.name),
                    &yours,
                );
            }
            if let Some((code, at)) = self.case.expected(index) {
                let reading = code::read_at(&mine, at[0], at[1]);
                if reading.code != Some(code) {
                    wrong.push(format!(
                        "frame {index} should be {code}, and reads as {}",
                        describe(&reading)
                    ));
                    self.save(report, &format!("preview-{index:04}-case.png"), &mine);
                }
            }
        }
        report.check(
            format!(
                "the previews are the same, byte for byte, as the {} twin's",
                twin.name
            ),
            unequal.is_empty(),
            named(&unequal, &format!("{} frames compared", twin.frames.len())),
        );
        report.check(
            "the previews show the source frames the export should",
            wrong.is_empty(),
            named(&wrong, "every code read"),
        );
        Ok(())
    }

    /// The paused monitor's frame at `time`, as the API hands it back.
    fn preview(&mut self, project: &str, time: f64) -> Result<Frame, String> {
        let reply = self.send(Request::PreviewFrame {
            path: project.to_owned(),
            time,
            output: None,
            width: None,
            height: None,
        })?;
        let Reply::Picture(picture) = reply else {
            return Err("preview.frame did not answer with a picture".to_owned());
        };
        let png = base64::engine::general_purpose::STANDARD
            .decode(picture.png)
            .map_err(|error| format!("the preview is not base64: {error}"))?;
        media::read_png(&png)
    }

    /// No export leaves its working files behind: the `.{stem}.concat-*`
    /// siblings it writes the picture, the sound and the mux to.
    fn check_leftovers(&mut self, report: &mut Report) {
        let mut found = Vec::new();
        let mut folders = vec![self.work.clone()];
        while let Some(folder) = folders.pop() {
            let Ok(entries) = std::fs::read_dir(&folder) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                if path.is_dir() {
                    folders.push(path);
                } else if name.starts_with('.') && name.contains(".concat-") {
                    found.push(path.display().to_string());
                }
            }
        }
        report.check(
            "the export leaves no working files behind",
            found.is_empty(),
            named(&found, "none"),
        );
    }

    /// Writes `frame` for the report, up to [`MAX_SAVED`] of them; a frame
    /// already written is not written again.
    fn save(&mut self, report: &mut Report, name: &str, frame: &Frame) {
        if self.saved >= MAX_SAVED || report.frames.iter().any(|saved| saved == name) {
            return;
        }
        let path = self.dir.join("frames").join(name);
        if media::save_frame(&path, frame).is_ok() {
            self.saved += 1;
            report.frames.push(name.to_owned());
        }
    }
}

/// Every frame of the file, in order.
fn decode(path: &Path) -> Result<Vec<Frame>, String> {
    let mut decoder = Decoder::open(path, &DecodeOptions::default())
        .map_err(|error| format!("the export does not open: {error}"))?;
    let mut frames = Vec::new();
    while let Some(frame) = decoder
        .next_frame()
        .map_err(|error| format!("the export does not decode: {error}"))?
    {
        frames.push(frame);
    }
    Ok(frames)
}

/// What a read found, in words.
fn describe(reading: &code::Reading) -> String {
    match reading.code {
        Some(code) => code.to_string(),
        None => {
            let means: Vec<String> = reading
                .means
                .iter()
                .map(|mean| format!("{mean:.0}"))
                .collect();
            format!("nothing (block means {})", means.join(" "))
        }
    }
}

/// The brightest channel inside `rect` of `frame`, or None for an empty
/// rectangle.
fn brightest(frame: &Frame, rect: &[u32; 4]) -> Option<u8> {
    let mut brightest = None;
    for y in rect[1]..rect[1] + rect[3] {
        for x in rect[0]..rect[0] + rect[2] {
            let [r, g, b, _] = frame.pixel(x, y)?;
            let value = r.max(g).max(b);
            brightest = Some(brightest.map_or(value, |so_far: u8| so_far.max(value)));
        }
    }
    brightest
}

/// A check's detail: the first few failures and how many more, or `fine`
/// when there were none.
fn named(failures: &[String], fine: &str) -> String {
    if failures.is_empty() {
        return fine.to_owned();
    }
    let mut detail = failures
        .iter()
        .take(MAX_NAMED)
        .cloned()
        .collect::<Vec<_>>()
        .join("; ");
    if failures.len() > MAX_NAMED {
        detail.push_str(&format!("; and {} more", failures.len() - MAX_NAMED));
    }
    detail
}

/// A path as the file system spells it, or as given where it cannot say.
fn canonical(path: &str) -> PathBuf {
    Path::new(path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(path))
}

/// `value` with every string that is exactly `$name` replaced by what
/// `name` is bound to. A name nothing is bound to is an error.
fn substitute(value: &Value, bindings: &BTreeMap<String, String>) -> Result<Value, String> {
    Ok(match value {
        Value::String(text) => match text.strip_prefix('$') {
            Some(rest) if rest.starts_with('$') => Value::String(rest.to_owned()),
            Some(name) => Value::String(
                bindings
                    .get(name)
                    .cloned()
                    .ok_or_else(|| format!("nothing is bound as {name}"))?,
            ),
            None => value.clone(),
        },
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| substitute(item, bindings))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, item)| Ok((key.clone(), substitute(item, bindings)?)))
                .collect::<Result<_, String>>()?,
        ),
        _ => value.clone(),
    })
}

/// Whether everything `given` says is in `read`, the request as the API
/// parsed it and wrote it back; the path of the first thing that is not.
/// Numbers are compared by value, since `0` and `0.0` are one number.
fn covered(given: &Value, read: &Value, path: &str) -> Result<(), String> {
    match (given, read) {
        (Value::Object(given), Value::Object(read)) => {
            for (key, value) in given {
                let at = format!("{path}.{key}");
                let other = read.get(key).ok_or_else(|| at.clone())?;
                covered(value, other, &at)?;
            }
            Ok(())
        }
        (Value::Array(given), Value::Array(read)) if given.len() == read.len() => {
            for (at, (value, other)) in given.iter().zip(read).enumerate() {
                covered(value, other, &format!("{path}[{at}]"))?;
            }
            Ok(())
        }
        (Value::Number(given), Value::Number(read)) if given.as_f64() == read.as_f64() => Ok(()),
        _ if given == read => Ok(()),
        _ => Err(path.to_owned()),
    }
}

/// The path of the first place `left` and `right` differ, or None when
/// they are equal.
fn difference(left: &Value, right: &Value, path: &str) -> Option<String> {
    match (left, right) {
        (Value::Object(left), Value::Object(right)) => {
            for key in left.keys().chain(right.keys()) {
                let at = format!("{path}.{key}");
                match (left.get(key), right.get(key)) {
                    (Some(a), Some(b)) => {
                        if let Some(found) = difference(a, b, &at) {
                            return Some(found);
                        }
                    }
                    _ => return Some(at),
                }
            }
            None
        }
        (Value::Array(left), Value::Array(right)) => {
            if left.len() != right.len() {
                return Some(format!(
                    "{path} (length {} against {})",
                    left.len(),
                    right.len()
                ));
            }
            left.iter()
                .zip(right)
                .enumerate()
                .find_map(|(at, (a, b))| difference(a, b, &format!("{path}[{at}]")))
        }
        _ if left == right => None,
        _ => Some(if path.is_empty() {
            "the top".to_owned()
        } else {
            path.to_owned()
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_misspelt_field_is_caught_and_a_default_left_out_is_not() {
        let given = json!({"method": "edit.apply", "path": "/p", "command": {"op": "splitClips", "clipIds": ["c1"], "time": 1.5}});
        let request: Request = serde_json::from_value(given.clone()).unwrap();
        let read = serde_json::to_value(&request).unwrap();
        assert_eq!(covered(&given, &read, ""), Ok(()));

        let misspelt = json!({"method": "edit.apply", "path": "/p", "command": {"op": "splitClips", "clipIds": ["c1"], "time": 1.5, "tme": 2}});
        let request: Request = serde_json::from_value(misspelt.clone()).unwrap();
        let read = serde_json::to_value(&request).unwrap();
        assert_eq!(
            covered(&misspelt, &read, ""),
            Err(".command.tme".to_owned())
        );
    }

    #[test]
    fn a_name_is_replaced_only_where_it_is_the_whole_string() {
        let bindings = BTreeMap::from([("a".to_owned(), "m1".to_owned())]);
        let value = json!({"x": "$a", "y": ["$a", "a$a"], "z": "$$a"});
        assert_eq!(
            substitute(&value, &bindings),
            Ok(json!({"x": "m1", "y": ["m1", "a$a"], "z": "$a"}))
        );
        assert!(substitute(&json!("$b"), &bindings).is_err());
    }

    #[test]
    fn the_first_difference_is_named_by_its_path() {
        let left = json!({"a": [1, {"b": 2}]});
        assert_eq!(difference(&left, &left, ""), None);
        assert_eq!(
            difference(&left, &json!({"a": [1, {"b": 3}]}), ""),
            Some(".a[1].b".to_owned())
        );
        assert_eq!(
            difference(&left, &json!({"a": [1, {"b": 2}], "c": 0}), ""),
            Some(".c".to_owned())
        );
    }
}
