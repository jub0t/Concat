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

use crate::case::{Case, Harness, Media, Shows, Splice};
use crate::code;
use crate::media;
use crate::report::Report;
use crate::sound;

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

/// How far a pip's onset in the export may be from where its source second
/// landed: a millisecond, well inside what a person hears as out of sync
/// and a twentieth of a frame at 50 fps.
const PIP_TOLERANCE: f64 = 0.001;

/// How far a tone's level may be from the one a case states: a hundredth
/// of full level, or a twentieth of the level itself where that is more
/// (0.4 dB). A fade read a frame early or late is off by more than both.
const LEVEL_TOLERANCE: f64 = 0.01;
const LEVEL_SHARE: f64 = 0.05;

/// How far a dissolve's mix may be from the share a case states, and half
/// how far apart its blocks may read it: a fiftieth. A frame early or late
/// in a one-second dissolve is a thirtieth off.
const BLEND_TOLERANCE: f64 = 0.02;

/// The commands that change what is looked at and not the edit: saved,
/// and never a step to undo.
const VIEW_STATE: [&str; 2] = ["selectTimeline", "moveTimeline"];

/// How many eight-bit levels a lossy codec may move a flat block by: the
/// bound on a dimmed picture read out of the export's file, where it is
/// more than [`BLEND_TOLERANCE`].
const ENCODED_LEVELS: f64 = 2.5;

/// How far from its place a pip is looked for: under half the gap
/// between two pips at the fastest speed a case uses.
const PIP_SEARCH: f64 = 0.15;

/// How much longer than the picture the decoded sound may run: less than
/// one AAC packet, 1024 samples. The export's file states the sound's
/// length exactly (its edit list says 4.000000 s for four seconds of
/// picture), but a packet is decoded whole, so the last one's padding
/// comes back with it: 9.3 ms of silence on a four-second export.
const SOUND_PACKET_SECONDS: f64 = 1024.0 / 48_000.0;

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
    // `CONCAT_CORPUS_KEEP` keeps a passing case's working files too: its
    // media, its project as saved and its export, to look at.
    if report.passed && std::env::var_os("CONCAT_CORPUS_KEEP").is_none() {
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
    if !case.expect.golden.is_empty() {
        runner.check_goldens(report, &project, cases)?;
    }
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
            sound,
        } => {
            let rate = FrameRate::new(Rational::new(rate[0], rate[1]));
            if !*sound {
                return media::clock(path, *width, *height, rate, *frames, *tag);
            }
            // The picture and the pips apart, then one file of both, as a
            // camera writes it.
            let picture = path.with_extension("picture.mov");
            let song = path.with_extension("sound.wav");
            media::clock(&picture, *width, *height, rate, *frames, *tag)?;
            let seconds = spec.sound_seconds().unwrap_or_default();
            sound::write_wav(&song, &sound::pips(*tag, seconds))?;
            concat_media::audio::mux(&picture, &song, path)
                .map_err(|error| format!("could not mux {}: {error}", path.display()))?;
            for part in [picture, song] {
                let _ = std::fs::remove_file(part);
            }
            Ok(())
        }
        Media::Still { tag, width, height } => media::still(path, *width, *height, *tag),
        Media::Pips { tag, seconds } => sound::write_wav(path, &sound::pips(*tag, *seconds)),
        Media::Tone { tag, seconds } => sound::write_wav(path, &sound::tone(*tag, *seconds)),
    }
}

/// What a session has to undo.
struct History {
    /// The document when nothing could be undone.
    base: Value,
    /// The edits that changed it since; `usize::MAX` when the case undid
    /// or redid some itself and the count is not this check's to make.
    edits: usize,
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
        // What there is to undo: the document as it was when nothing could
        // be undone, and the edits made since. Held to before every save
        // and reopen, which is where a session's history ends.
        let mut history = History {
            base: self.document(&project)?,
            edits: 0,
        };
        for (at, step) in self.case.steps.iter().enumerate() {
            let alias = step.get("as").and_then(Value::as_str);
            if alias.is_some_and(|alias| skip.iter().any(|name| name == alias)) {
                continue;
            }
            if step.get("step").is_some() {
                let harness: Harness = serde_json::from_value(step.clone())
                    .map_err(|error| format!("step {at} is no harness step: {error}"))?;
                self.undo_and_redo(report, label, &project, &format!("step {at}"), &history)?;
                self.harness(report, label, &project, at, &harness)?;
                history = History {
                    base: self.document(&project)?,
                    edits: 0,
                };
                continue;
            }
            let before = self.document(&project)?;
            self.script_step(at, step, &mut bindings)?;
            let after = self.document(&project)?;
            match step.get("method").and_then(Value::as_str) {
                // The case's own undo or redo: what it leaves to undo is
                // not counted here, only that all of it undoes and redoes.
                Some("edit.undo" | "edit.redo") => history.edits = usize::MAX,
                // Whatever cannot be undone moves the start.
                _ if !self.can_undo(&project)? => {
                    history = History {
                        base: after,
                        edits: 0,
                    };
                }
                // Which timeline is showing and the order of their tabs
                // are saved with the document and are no step to undo:
                // what the person is looking at, not the edit.
                Some("edit.apply")
                    if step["command"]["op"]
                        .as_str()
                        .is_some_and(|op| VIEW_STATE.contains(&op)) => {}
                // An edit or an import that changed the document is one
                // step to undo; one that changed nothing is none.
                _ if after != before && history.edits != usize::MAX => history.edits += 1,
                _ => {}
            }
        }
        self.undo_and_redo(report, label, &project, "the end", &history)?;
        self.round_trip(report, label, &project, "the end", |_| Ok(()))?;
        Ok(project)
    }

    /// Whether the project has anything to undo.
    fn can_undo(&mut self, project: &str) -> Result<bool, String> {
        match self.send(Request::ProjectGet {
            path: project.to_owned(),
        })? {
            Reply::View(view) => Ok(view.can_undo),
            _ => Err("project.get did not answer with the project".to_owned()),
        }
    }

    /// Undoes everything there is to undo and redoes all of it: the
    /// document goes back to where the history began, in as many steps as
    /// edits were made, and comes forward to exactly what it was.
    fn undo_and_redo(
        &mut self,
        report: &mut Report,
        label: &str,
        project: &str,
        when: &str,
        history: &History,
    ) -> Result<(), String> {
        let ended = self.document(project)?;
        let mut undone = 0usize;
        while self.can_undo(project)? {
            self.send(Request::EditUndo {
                path: project.to_owned(),
            })?;
            undone += 1;
            if undone > 10_000 {
                return Err(format!("{label}: undo never ran out at {when}"));
            }
        }
        if undone == 0 {
            return Ok(());
        }
        let began = self.document(project)?;
        for _ in 0..undone {
            self.send(Request::EditRedo {
                path: project.to_owned(),
            })?;
        }
        // Which timeline is showing and the order of the tabs are no step
        // of undo: walking back past the edit that made a timeline lets go
        // of its place among them. The walk is this check's own, so it puts
        // the view back as the case left it, by the commands a person has.
        let tabs = ended["view"]["tabs"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let showing = ended["view"]["showing"].as_str().map(str::to_owned);
        let mut view = Vec::new();
        for (index, tab) in tabs.iter().enumerate() {
            view.push(json!({ "op": "moveTimeline", "timelineId": tab, "index": index }));
        }
        if let Some(showing) = showing {
            view.push(json!({ "op": "selectTimeline", "timelineId": showing }));
        }
        for command in view {
            let request: Request = serde_json::from_value(json!({
                "method": "edit.apply",
                "path": project,
                "command": command,
            }))
            .map_err(|error| format!("{label}: the view's command is no request: {error}"))?;
            self.send(request)?;
        }
        let again = self.document(project)?;
        let counted = history.edits != usize::MAX;
        let mut wrong = Vec::new();
        // The count of ids made is saved with the document and never goes
        // back, so that an id is never made twice: an undo leaves it where
        // the edits took it, and it is no part of what is compared.
        let edit = |document: &Value| {
            let mut edit = document.clone();
            if let Some(fields) = edit.as_object_mut() {
                fields.remove("mint");
            }
            edit
        };
        if let Some(path) = difference(&edit(&history.base), &edit(&began), "") {
            wrong.push(format!(
                "undone, the document is not what it was before the edits: it first differs at {path}"
            ));
        }
        if let Some(path) = difference(&ended, &again, "") {
            wrong.push(format!(
                "redone, the document is not what the edits left: it first differs at {path}"
            ));
        }
        if counted && undone != history.edits {
            wrong.push(format!("{} edits took {undone} undos", history.edits));
        }
        report.check(
            format!(
                "{label}: every edit undoes in one step and redoes to the same document, at {when}"
            ),
            wrong.is_empty(),
            named(&wrong, &format!("{undone} steps back and forward")),
        );
        Ok(())
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
        let pointers = value
            .as_object_mut()
            .and_then(|object| object.remove("bind"))
            .map(|bind| {
                serde_json::from_value::<BTreeMap<String, String>>(bind)
                    .map_err(|_| format!("step {at}: \"bind\" is names to JSON pointers"))
            })
            .transpose()?
            .unwrap_or_default();
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
        if alias.is_none() && pointers.is_empty() {
            return Ok(());
        }
        let Reply::View(view) = reply else {
            return Err(format!(
                "step {at}: {method} answered with no project to bind from"
            ));
        };
        if let Some(alias) = alias {
            let id = match method.as_str() {
                // The bin's id for the file: the one it has already, for a
                // file imported before.
                "media.import" => view.created_id.clone().ok_or_else(|| {
                    let file = value["file"].as_str().unwrap_or_default();
                    format!("step {at}: {file} is not in the bin after its import")
                })?,
                "edit.apply" => view.created_id.clone().ok_or_else(|| {
                    format!("step {at}: the edit minted nothing to bind as {alias}")
                })?,
                _ => {
                    return Err(format!(
                        "step {at}: only media.import and edit.apply bind a name, not {method}"
                    ));
                }
            };
            if bindings.insert(alias.clone(), id).is_some() {
                return Err(format!("step {at}: {alias} is bound already"));
            }
        }
        if pointers.is_empty() {
            return Ok(());
        }
        // After the step's own name, so a pointer can go through the record
        // of what the step made: `/project/items/$clip/lane`.
        let answer = serde_json::to_value(&view)
            .map_err(|error| format!("step {at}: the answer does not encode: {error}"))?;
        for (name, pointer) in pointers {
            let pointer =
                named_pointer(&pointer, bindings).map_err(|error| format!("step {at}: {error}"))?;
            let found = match answer.pointer(&pointer) {
                Some(Value::String(text)) => text.clone(),
                Some(Value::Number(number)) => number.to_string(),
                _ => {
                    return Err(format!(
                        "step {at}: {pointer} is no string or number in the answer"
                    ));
                }
            };
            if bindings.insert(name.clone(), found).is_some() {
                return Err(format!("step {at}: {name} is bound already"));
            }
        }
        Ok(())
    }

    /// One of the harness's own steps.
    fn harness(
        &mut self,
        report: &mut Report,
        label: &str,
        project: &str,
        at: usize,
        step: &Harness,
    ) -> Result<(), String> {
        let when = format!("step {at}");
        match step {
            Harness::CloseAndReopen => self.round_trip(report, label, project, &when, |_| Ok(())),
            Harness::RemoveFile { media } | Harness::RestoreFile { media } => {
                let file = self
                    .files
                    .get(media)
                    .cloned()
                    .ok_or_else(|| format!("step {at}: no file was made for {media}"))?;
                let away = file.with_extension("away");
                let (from, to, what) = match step {
                    Harness::RemoveFile { .. } => (file, away, "gone"),
                    _ => (away, file, "back"),
                };
                let when = format!("{when}, with {media}'s file {what}");
                self.round_trip(report, label, project, &when, |_| {
                    std::fs::rename(&from, &to).map_err(|error| {
                        format!(
                            "could not move {} to {}: {error}",
                            from.display(),
                            to.display()
                        )
                    })
                })
            }
            Harness::SpliceDocument { merge } => self.splice(report, label, project, &when, merge),
        }
    }

    /// Saves and closes the project, writes `merge` into its
    /// saved document, reopens it, and checks all of it is in the document
    /// the project then gives: what this build cannot read, it keeps.
    fn splice(
        &mut self,
        report: &mut Report,
        label: &str,
        project: &str,
        when: &str,
        merge: &[Splice],
    ) -> Result<(), String> {
        self.send(Request::ProjectClose {
            path: project.to_owned(),
            save: true,
        })?;
        let file = Path::new(project).join(concat_host::session::DOCUMENT);
        let text = std::fs::read_to_string(&file)
            .map_err(|error| format!("could not read {}: {error}", file.display()))?;
        let mut document: Value = serde_json::from_str(&text)
            .map_err(|error| format!("{} is not JSON: {error}", file.display()))?;
        for splice in merge {
            let object = document
                .pointer_mut(&splice.at)
                .and_then(Value::as_object_mut)
                .ok_or_else(|| {
                    format!("{when}: {} is no object in the saved document", splice.at)
                })?;
            for (key, value) in splice.value.as_object().into_iter().flatten() {
                object.insert(key.clone(), value.clone());
            }
        }
        let text = serde_json::to_string_pretty(&document)
            .map_err(|error| format!("the spliced document does not encode: {error}"))?;
        std::fs::write(&file, text)
            .map_err(|error| format!("could not write {}: {error}", file.display()))?;
        self.send(Request::ProjectOpen {
            path: project.to_owned(),
        })?;
        let opened = self.document(project)?;
        let mut lost = Vec::new();
        for splice in merge {
            for (key, value) in splice.value.as_object().into_iter().flatten() {
                if opened
                    .pointer(&splice.at)
                    .and_then(|object| object.get(key))
                    != Some(value)
                {
                    lost.push(format!("{} lost its {key}", splice.at));
                }
            }
        }
        report.check(
            format!(
                "{label}: what was written into the saved document at {when} is in the opened one"
            ),
            lost.is_empty(),
            named(&lost, &format!("all {} of it", merge.len())),
        );
        Ok(())
    }

    /// Saves and closes the project, does `between` to what is on disk,
    /// reopens it and checks the document came back as it went.
    fn round_trip(
        &mut self,
        report: &mut Report,
        label: &str,
        project: &str,
        when: &str,
        between: impl FnOnce(&Path) -> Result<(), String>,
    ) -> Result<(), String> {
        let before = self.document(project)?;
        self.send(Request::ProjectClose {
            path: project.to_owned(),
            save: true,
        })?;
        between(Path::new(project))?;
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
        let mut spec = match &self.case.export {
            None => ExportSpec::default(),
            Some(given) => {
                // The output is the runner's to name; a spec does not read
                // without one.
                let mut whole = given.clone();
                if let Some(object) = whole.as_object_mut() {
                    object.insert("output".to_owned(), Value::String(String::new()));
                }
                let spec: ExportSpec = serde_json::from_value(whole)
                    .map_err(|error| format!("the case's export is no export spec: {error}"))?;
                let read = serde_json::to_value(&spec)
                    .map_err(|error| format!("the export spec does not encode: {error}"))?;
                covered(given, &read, "")
                    .map_err(|path| format!("export{path} is not a field the spec reads"))?;
                spec
            }
        };
        spec.output = output.to_string_lossy().into_owned();
        let reply = self.send(Request::ExportRun {
            path: project.to_owned(),
            spec,
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
        // The file as the case says the export makes it, or as the
        // timeline is.
        let (file_frames, file_width, file_height, file_rate) = match &expect.file {
            Some(file) => (file.frames, file.width, file.height, file.rate),
            None => (expect.frames, expect.width, expect.height, expect.rate),
        };
        let wanted = Rational::new(file_rate[0], file_rate[1]);
        report.check(
            format!(
                "the file is {file_width}×{file_height} at {}/{} fps",
                file_rate[0], file_rate[1]
            ),
            video.width == file_width && video.height == file_height && rate == wanted,
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
                .document()
                .map_err(|error| format!("project.get answered with no document: {error}"))?
                .showing_end()
                .display_seconds(),
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
            format!("the file holds {file_frames} frames"),
            frames.len() == file_frames as usize,
            format!("it holds {}", frames.len()),
        );

        let mut wrong = Vec::new();
        let mut worst: f64 = 0.0;
        for index in 0..file_frames {
            // The timeline frame this file frame is of, and how much
            // larger the file draws it.
            let (of, times) = self.case.file_frame(index);
            let shows = self
                .case
                .expected(of)
                .ok_or_else(|| format!("no run covers frame {of}"))?;
            let Some(frame) = frames.get(index as usize) else {
                wrong.push(format!("frame {index} is missing"));
                continue;
            };
            let insets = self.case.expected_insets(of);
            for shows in std::iter::once(shows).chain(insets) {
                if let Some(found) = shown_wrong(frame, enlarged(shows, times), &mut worst, true) {
                    wrong.push(format!("frame {index} {found}"));
                    self.save(report, &format!("export-{index:04}.png"), frame);
                }
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

        // The monitor, paused on each frame's own instant, shows what the
        // export wrote there: the picture a person edits by is the one
        // they get.
        let fps = self.case.fps();
        let mut unlike = Vec::new();
        for index in 0..expect.frames {
            let Some(shows) = self.case.expected(index) else {
                continue;
            };
            let seen = self.preview(project, f64::from(index) / fps)?;
            let insets = self.case.expected_insets(index);
            for shows in std::iter::once(shows).chain(insets) {
                if let Some(found) = shown_wrong(&seen, shows, &mut 0.0, false) {
                    unlike.push(format!("frame {index} {found}"));
                    self.save(report, &format!("monitor-{index:04}.png"), &seen);
                }
            }
        }
        report.check(
            "the monitor shows at every frame what the export should",
            unlike.is_empty(),
            named(&unlike, &format!("{} frames seen", expect.frames)),
        );

        if !expect.sound.is_empty() {
            self.check_sound(report, output)?;
        }

        if !expect.level.is_empty() {
            self.check_levels(report, output)?;
        }

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

    /// Holds the export's sound to the pips the case's sound runs put in
    /// it: each one heard, at its pitch, within [`PIP_TOLERANCE`] of where
    /// its source second landed, and nothing heard anywhere else.
    fn check_sound(&mut self, report: &mut Report, output: &Path) -> Result<(), String> {
        let options = concat_media::AudioOptions {
            rate: sound::RATE,
            channels: 1,
            format: concat_media::SampleFormat::F32,
            ..concat_media::AudioOptions::default()
        };
        let samples = concat_media::AudioDecoder::open(output, &options)
            .and_then(|mut decoder| decoder.collect_f32())
            .map_err(|error| format!("the export's sound does not decode: {error}"))?;
        let length = samples.len() as f64 / f64::from(sound::RATE);
        let wanted = f64::from(self.case.expect.frames) / self.case.fps();
        report.check(
            "the sound lasts as long as the picture, to the codec's last packet",
            length >= wanted - 0.5 / f64::from(sound::RATE)
                && length < wanted + SOUND_PACKET_SECONDS,
            format!("{length:.4} s of sound for {wanted:.4} s of picture"),
        );

        let heard = sound::hear(&samples, sound::RATE);
        let expected = self.case.expected_pips();
        let runs = &self.case.expect.sound;
        let mut wrong = Vec::new();
        let mut worst: f64 = 0.0;
        let mut run_worst = vec![0.0f64; runs.len()];
        let mut used = vec![false; heard.len()];
        for pip in &expected {
            let allowed = runs[pip.run]
                .known
                .as_ref()
                .map_or(PIP_TOLERANCE, |known| known.within);
            let nearest = heard
                .iter()
                .enumerate()
                .filter(|(index, found)| !used[*index] && (found.at - pip.at).abs() <= PIP_SEARCH)
                .min_by(|a, b| (a.1.at - pip.at).abs().total_cmp(&(b.1.at - pip.at).abs()));
            let Some((index, found)) = nearest else {
                wrong.push(format!(
                    "second {} of media {} should start at {:.4} s, and nothing is heard there",
                    pip.second, pip.tag, pip.at
                ));
                continue;
            };
            used[index] = true;
            let off = found.at - pip.at;
            worst = worst.max(off.abs());
            run_worst[pip.run] = run_worst[pip.run].max(off.abs());
            if off.abs() > allowed {
                wrong.push(format!(
                    "second {} of media {} is heard {:.1} ms {} at {:.4} s; {:.1} ms is allowed",
                    pip.second,
                    pip.tag,
                    off.abs() * 1000.0,
                    if off < 0.0 { "early" } else { "late" },
                    found.at,
                    allowed * 1000.0
                ));
            }
            if found.tag != pip.tag {
                wrong.push(format!(
                    "at {:.4} s, second {} of media {} sounds at media {}'s pitch",
                    found.at, pip.second, pip.tag, found.tag
                ));
            }
        }
        for (found, _) in heard.iter().zip(&used).filter(|(_, used)| !**used) {
            wrong.push(format!(
                "a pip of media {} at {:.4} s that no sound run puts there",
                found.tag, found.at
            ));
        }
        for (index, run) in runs.iter().enumerate() {
            let Some(known) = &run.known else {
                continue;
            };
            let heard_any = expected.iter().any(|pip| pip.run == index);
            report.check(
                format!(
                    "the sound run at frames {:?} still has its known flaw",
                    run.frames
                ),
                heard_any && run_worst[index] > PIP_TOLERANCE,
                format!(
                    "off by up to {:.1} ms, known up to {:.1} ms: {}{}",
                    run_worst[index] * 1000.0,
                    known.within * 1000.0,
                    known.why,
                    if run_worst[index] <= PIP_TOLERANCE {
                        "; it is within a millisecond now, so take the known bound out"
                    } else {
                        ""
                    }
                ),
            );
        }
        report.sound_worst = worst;
        report.check(
            format!(
                "the sound carries the {} pips the case expects, where it expects them",
                expected.len()
            ),
            wrong.is_empty(),
            named(
                &wrong,
                &format!(
                    "{} pips heard, the furthest {:.2} ms from its place",
                    heard.len(),
                    worst * 1000.0
                ),
            ),
        );
        Ok(())
    }

    /// Holds the monitor's frames to the case's stored pictures: what the
    /// compositor drew, before any encoder, judged as [`crate::golden`]
    /// judges. A failed picture's page is `golden/index.html` beside the
    /// report.
    fn check_goldens(
        &mut self,
        report: &mut Report,
        project: &str,
        cases: &Path,
    ) -> Result<(), String> {
        use crate::golden::{Family, Golden, Mode, Rgba8Image};

        let (backend, adapter) = self.api.adapter().ok_or_else(|| {
            "the case has golden pictures and there is no graphics adapter to draw them".to_owned()
        })?;
        let family = Family::of_adapter(&backend, &adapter);
        let out = self.dir.parent().unwrap_or(&self.dir).to_path_buf();
        let mode = Mode::from_env(self.name, &out.join("bless.md"))?;
        let folder = self.dir.join("golden");
        let golden = Golden::new(&cases.join(self.name), family, &folder, mode);
        let fps = self.case.fps();
        let mut results = Vec::new();
        for wanted in &self.case.expect.golden {
            let seen = self.preview(project, f64::from(wanted.frame) / fps)?;
            let image = Rgba8Image::from_frame(&seen)
                .ok_or_else(|| format!("the monitor's frame {} is no picture", wanted.frame))?;
            let result = golden.check(&wanted.name, &image);
            let mut check = result.to_check();
            check.detail = format!("{} ({adapter}, {family})", check.detail);
            report.checks.push(check);
            results.push(result);
        }
        if results.iter().any(|result| !result.ok()) {
            crate::golden::write_failure_page(&folder, &results)?;
        }
        Ok(())
    }

    /// Holds each tone to how loud the case says it is at each instant.
    fn check_levels(&mut self, report: &mut Report, output: &Path) -> Result<(), String> {
        let options = concat_media::AudioOptions {
            rate: sound::RATE,
            channels: 1,
            format: concat_media::SampleFormat::F32,
            ..concat_media::AudioOptions::default()
        };
        let samples = concat_media::AudioDecoder::open(output, &options)
            .and_then(|mut decoder| decoder.collect_f32())
            .map_err(|error| format!("the export's sound does not decode: {error}"))?;
        let mut wrong = Vec::new();
        let mut worst: f64 = 0.0;
        for level in &self.case.expect.level {
            let tag = self.case.media.get(&level.media).map_or(0, Media::tag);
            let heard = sound::level(&samples, sound::RATE, level.at, tag);
            let off = (heard - level.gain).abs();
            worst = worst.max(off);
            if off > LEVEL_TOLERANCE.max(level.gain * LEVEL_SHARE) {
                wrong.push(format!(
                    "{} at {} s is at {heard:.4} of its level, not {}",
                    level.media, level.at, level.gain
                ));
            }
        }
        report.check(
            format!(
                "the sound is as loud as the case expects at its {} instants",
                self.case.expect.level.len()
            ),
            wrong.is_empty(),
            named(&wrong, &format!("the furthest {worst:.4} from its level")),
        );
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
        let (ours, theirs) = (edit_of(&ours), edit_of(&theirs));
        let differ = difference(&ours, &theirs, "").is_some();
        if twin.same {
            report.check(
                format!("the case's timeline is the {} twin's again", twin.name),
                !differ,
                match difference(&ours, &theirs, "") {
                    None => "entry for entry".to_owned(),
                    Some(path) => format!("they first differ at {path}"),
                },
            );
        } else {
            report.check(
                format!("the {} twin's timeline differs from the case's", twin.name),
                differ,
                if differ {
                    "it does, so the comparison is of two different edits".to_owned()
                } else {
                    "they are the same edit, which would make the comparison prove nothing"
                        .to_owned()
                },
            );
        }

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
            if let Some(shows) = self.case.expected(index)
                && let Some(found) = shown_wrong(&mine, shows, &mut 0.0, false)
            {
                wrong.push(format!("frame {index} {found}"));
                self.save(report, &format!("preview-{index:04}-case.png"), &mine);
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

/// What is wrong with `frame` if it does not show `shows`, in words that
/// follow "frame N"; `worst` keeps the furthest a code block was from its
/// level.
///
/// `encoded` says the frame came out of the export's file and not from the
/// monitor: an eight-bit picture a lossy codec has had. A dimmed picture's
/// light is then known no better than the codec keeps its levels, which
/// near white is worth more light a level than the usual bound allows -
/// one encoder left a frame a fortieth dimmer than the last as it was,
/// two levels' worth - so there the bound is two and a half levels, and
/// the monitor's frame, drawn before any codec, is what is held tightly.
fn shown_wrong(frame: &Frame, shows: Shows, worst: &mut f64, encoded: bool) -> Option<String> {
    match shows {
        Shows::Code(code, at, block) => {
            let reading = code::read_sized(frame, at[0], at[1], block);
            *worst = worst.max(reading.worst);
            (reading.code != Some(code))
                .then(|| format!("should be {code}, and reads as {}", describe(&reading)))
        }
        Shows::Either(first, second, at, block) => {
            let reading = code::read_sized(frame, at[0], at[1], block);
            *worst = worst.max(reading.worst);
            (reading.code != Some(first) && reading.code != Some(second)).then(|| {
                format!(
                    "should be {first} or the frame after, and reads as {}",
                    describe(&reading)
                )
            })
        }
        Shows::Faded(code, share, at, block) => {
            // What one level of the white blocks is worth in light, at the
            // level this share of it shows as.
            let level = f64::from(code::HIGH) * share.max(1e-6).powf(1.0 / 2.4);
            let allowed = if encoded {
                BLEND_TOLERANCE.max(ENCODED_LEVELS * 2.4 * share / level)
            } else {
                BLEND_TOLERANCE
            };
            match code::read_faded(frame, at[0], at[1], block, code) {
                None => Some(format!("should be {code} dimmed, and is not that")),
                Some((found, spread)) => ((found - share).abs() > allowed
                    || spread > 2.0 * allowed)
                    .then(|| {
                        format!(
                            "should show {share:.3} of {code}, and shows {found:.3} (its blocks {spread:.3} apart)"
                        )
                    }),
            }
        }
        Shows::Blend(first, second, share, at, block) => {
            // At either end the picture is all but one of the two, and
            // reads as it; in between it is held to its share.
            let whole = code::read_sized(frame, at[0], at[1], block).code;
            if (share <= BLEND_TOLERANCE && whole == Some(first))
                || (share >= 1.0 - BLEND_TOLERANCE && whole == Some(second))
            {
                return None;
            }
            match code::read_blend(frame, at[0], at[1], block, first, second) {
                None => Some(format!(
                    "should be {first} melting into {second}, and is not those two alone"
                )),
                Some((found, spread)) => ((found - share).abs() > BLEND_TOLERANCE
                    || spread > 2.0 * BLEND_TOLERANCE)
                    .then(|| {
                        format!(
                            "should be {share:.3} of the way from {first} into {second}, and is {found:.3} (its blocks {spread:.3} apart)"
                        )
                    }),
            }
        }
        Shows::Black => {
            let all = [0, 0, frame.width(), frame.height()];
            brightest(frame, &all)
                .filter(|brightest| *brightest > BLACK)
                .map(|brightest| format!("should be black, and reaches {brightest}"))
        }
    }
}

/// `shows` as a picture `times` larger draws it.
fn enlarged(shows: Shows, times: u32) -> Shows {
    let grow = |at: [u32; 2]| [at[0] * times, at[1] * times];
    match shows {
        Shows::Code(code, at, block) => Shows::Code(code, grow(at), block * times),
        Shows::Either(first, second, at, block) => {
            Shows::Either(first, second, grow(at), block * times)
        }
        Shows::Faded(code, share, at, block) => Shows::Faded(code, share, grow(at), block * times),
        Shows::Blend(first, second, share, at, block) => {
            Shows::Blend(first, second, share, grow(at), block * times)
        }
        Shows::Black => Shows::Black,
    }
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

/// A JSON pointer with each part that is `$name` put as what is bound to
/// that name.
fn named_pointer(pointer: &str, bindings: &BTreeMap<String, String>) -> Result<String, String> {
    let parts: Vec<String> = pointer
        .split('/')
        .map(|part| match part.strip_prefix('$') {
            Some(name) => bindings
                .get(name)
                .cloned()
                .ok_or_else(|| format!("nothing is bound as {name}")),
            None => Ok(part.to_owned()),
        })
        .collect::<Result<_, _>>()?;
    Ok(parts.join("/"))
}

/// The edit of a saved document: its timelines, their lanes and what is on
/// them, without the bin, the view and the count of ids made.
fn edit_of(document: &Value) -> Value {
    json!({
        "sequences": document["sequences"],
        "lanes": document["lanes"],
        "items": document["items"],
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
