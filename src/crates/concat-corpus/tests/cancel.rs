// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Cancel, at every stage of an export: it lets go within a second, leaves
//! nothing half-written, leaves the file that was there before as it was,
//! and the next export starts.
//!
//! The timeline is ten minutes of two tones under a black picture at ten
//! frames a second: small enough to draw quickly on a software adapter,
//! long enough that its sound takes seconds to mix, which a Cancel used to
//! wait out. The sound mixes alongside the picture from the export's start,
//! so a cancel at its first sign of life or half way through the picture
//! lands in the mix as well. The export is run once to its end, so there is
//! a finished file where the cancelled ones would write theirs; then it is
//! run again and cancelled at a chosen moment - its first sign of life,
//! half way through the picture, in what is left of the mix once the
//! picture is drawn, as the two are joined - and held each time to the same
//! four things.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use concat_api::{Api, AppDirs, ErrorCode, Event, Reply, Request, Response};
use concat_corpus::sound;
use serde_json::{Value, json};

/// How long a cancelled export has to end.
const LET_GO: Duration = Duration::from_secs(1);

/// How long any export here may take before the test calls it stuck.
const STUCK: Duration = Duration::from_secs(600);

/// The timeline's length, in seconds.
const SECONDS: f64 = 600.0;

/// When to cancel.
#[derive(Clone, Copy, Debug)]
enum At {
    /// On the export's first progress.
    FirstSign,
    /// Once half the picture is drawn.
    HalfThePicture,
    /// On the first progress of the stage of this name.
    Stage(&'static str),
}

struct Bench {
    api: Api,
    events: Receiver<Event>,
    project: String,
    output: PathBuf,
}

impl Bench {
    fn send(&mut self, request: Value) -> Reply {
        let request: Request = serde_json::from_value(request).expect("a request");
        match self.api.dispatch(request) {
            Response::Result(reply) => reply,
            Response::Error(error) => panic!("refused ({:?}): {}", error.code, error.message),
        }
    }

    /// Starts an export to the bench's output; its job.
    fn start(&mut self) -> String {
        // Whatever an earlier export still had to say.
        while self.events.try_recv().is_ok() {}
        let reply = self.send(json!({
            "method": "export.run",
            "path": self.project,
            "output": self.output.to_string_lossy(),
        }));
        match reply {
            Reply::Started(started) => started.job,
            _ => panic!("export.run did not start a job"),
        }
    }

    /// Waits for `job` to end; whether it finished, or the error it ended
    /// with.
    fn end(&mut self, job: &str) -> Result<(), concat_api::ApiError> {
        let deadline = Instant::now() + STUCK;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.events.recv_timeout(left) {
                Ok(Event::ExportDone { job: done, .. }) if done == job => return Ok(()),
                Ok(Event::ExportFailed {
                    job: failed, error, ..
                }) if failed == job => {
                    return Err(error);
                }
                Ok(_) => {}
                Err(_) => panic!("the export did not end in {} s", STUCK.as_secs()),
            }
        }
    }

    /// Starts an export, cancels it at `at`, and says how long it took to
    /// end after the cancel and how it ended. None when it finished before
    /// the moment came.
    fn cancel_at(&mut self, at: At) -> Option<(Duration, Result<(), concat_api::ApiError>)> {
        let job = self.start();
        let deadline = Instant::now() + STUCK;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.events.recv_timeout(left) {
                Ok(Event::ExportProgress {
                    job: moving,
                    frame,
                    total,
                    stage,
                    ..
                }) if moving == job => {
                    let now = match at {
                        At::FirstSign => true,
                        At::HalfThePicture => total > 0 && frame * 2 >= total,
                        At::Stage(name) => stage == name,
                    };
                    if now {
                        break;
                    }
                }
                Ok(Event::ExportDone { job: done, .. }) if done == job => return None,
                Ok(Event::ExportFailed {
                    job: failed, error, ..
                }) if failed == job => {
                    panic!(
                        "the export failed before it was cancelled: {}",
                        error.message
                    );
                }
                Ok(_) => {}
                Err(_) => panic!("the export did not reach {at:?} in {} s", STUCK.as_secs()),
            }
        }
        let asked = Instant::now();
        self.send(json!({ "method": "export.cancel", "job": job }));
        let ended = self.end(&job);
        Some((asked.elapsed(), ended))
    }

    /// The export's working files still on disk: the dot-named siblings it
    /// writes the picture, the sound and the join to.
    fn leftovers(&self) -> Vec<String> {
        let mut found = Vec::new();
        let mut folders = vec![PathBuf::from(&self.project)];
        while let Some(folder) = folders.pop() {
            for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if entry.path().is_dir() {
                    folders.push(entry.path());
                } else if name.starts_with('.') && name.contains(".concat-") {
                    found.push(name);
                }
            }
        }
        found
    }
}

/// A project of two ten-minute tones, one over the other, at ten frames a
/// second, in a scratch folder under `target/corpus/cancel`.
fn bench() -> Bench {
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let dir = tmp.parent().unwrap_or(&tmp).join("corpus").join("cancel");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("app")).expect("a scratch folder");
    let dir = std::path::absolute(&dir).expect("an absolute path");
    let tones: Vec<PathBuf> = [1u8, 3]
        .iter()
        .map(|tag| {
            let path = dir.join(format!("tone-{tag}.wav"));
            sound::write_wav(&path, &sound::tone(*tag, SECONDS)).expect("a tone");
            path
        })
        .collect();

    let (sender, events) = mpsc::channel();
    let mut api = Api::with_dirs(
        AppDirs::under(&dir.join("app")),
        Arc::new(move |event| {
            let _ = sender.send(event);
        }),
    );
    api.restrict_writes_to(vec![dir.clone()]);
    let mut bench = Bench {
        api,
        events,
        project: String::new(),
        output: PathBuf::new(),
    };
    bench.send(json!({
        "method": "project.create",
        "location": dir.join("project").to_string_lossy(),
        "name": "cancel",
        "video": { "width": 320, "height": 180, "rateNum": 10, "rateDen": 1 },
    }));
    bench.project = bench
        .api
        .recents()
        .into_iter()
        .map(|info| info.path)
        .next()
        .expect("the project is in the recents");
    bench.output = Path::new(&bench.project).join("cancel.mp4");
    for tone in &tones {
        let reply = bench.send(json!({
            "method": "media.import",
            "path": bench.project,
            "file": tone.to_string_lossy(),
        }));
        let Reply::View(view) = reply else {
            panic!("media.import answered with no project");
        };
        let id = view
            .project
            .media
            .last()
            .expect("the tone is in the bin")
            .id
            .clone();
        bench.send(json!({
            "method": "edit.apply",
            "path": bench.project,
            "command": { "op": "addClipAtFirstFree", "mediaId": id, "start": 0 },
        }));
    }
    bench
}

#[test]
fn a_cancelled_export_lets_go_within_a_second_at_every_stage() {
    let mut bench = bench();

    // To its end once: the file a cancelled export must leave alone.
    let began = Instant::now();
    let job = bench.start();
    bench
        .end(&job)
        .expect("the export finishes when left alone");
    let whole = std::fs::read(&bench.output).expect("the finished file");
    eprintln!(
        "left alone: {:.1} s for {} bytes",
        began.elapsed().as_secs_f64(),
        whole.len()
    );
    assert!(bench.leftovers().is_empty(), "{:?}", bench.leftovers());

    let mut slow = Vec::new();
    for at in [
        At::FirstSign,
        At::HalfThePicture,
        At::Stage("mixing audio"),
        At::Stage("muxing"),
    ] {
        let Some((took, ended)) = bench.cancel_at(at) else {
            // The join is short enough to be over before a cancel lands,
            // and the mix comes as a stage of its own only when it outlasts
            // the picture; anywhere else the moment was missed.
            assert!(
                matches!(at, At::Stage("muxing" | "mixing audio")),
                "the export finished before {at:?} came"
            );
            eprintln!("{at:?}: finished before the cancel landed");
            continue;
        };
        eprintln!("{at:?}: ended {:.3} s after the cancel", took.as_secs_f64());
        match ended {
            // Too late to stop: it finished, and its file is whole.
            Ok(()) => assert!(
                matches!(at, At::Stage("muxing")),
                "cancelled at {at:?}, the export finished all the same"
            ),
            Err(error) => assert_eq!(
                error.code,
                ErrorCode::Cancelled,
                "cancelled at {at:?}, the export says it failed: {}",
                error.message
            ),
        }
        if took > LET_GO {
            slow.push(format!("{at:?}: {:.2} s", took.as_secs_f64()));
        }
        assert!(
            bench.leftovers().is_empty(),
            "cancelled at {at:?}, working files are left: {:?}",
            bench.leftovers()
        );
        let after = std::fs::read(&bench.output).expect("the file that was there is still there");
        assert!(
            after == whole || matches!(at, At::Stage("muxing")),
            "cancelled at {at:?}, the finished file that was there has changed"
        );
    }
    assert!(
        slow.is_empty(),
        "a cancelled export took over {} s to let go: {}",
        LET_GO.as_secs(),
        slow.join("; ")
    );

    // And the next one starts, and finishes.
    let job = bench.start();
    bench
        .end(&job)
        .expect("an export after the cancelled ones finishes");
}
