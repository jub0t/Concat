// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The engine's services, started once, and the bridge between their
//! threads and the window's.
//!
//! Slint's models and properties may only be touched on the event-loop
//! thread. Everything slow - probes, decodes, renders, downloads - runs on
//! a worker through [`spawn`], [`spawn_long`] or [`spawn_save`] - one lane
//! each for the monitor's work, the long jobs and the saves, so none waits
//! behind another - and hands its result back with
//! `slint::invoke_from_event_loop`, where [`Shell::with`] reaches the window
//! and its state again. The state itself never leaves the event-loop thread,
//! which is why it can live in a plain `RefCell` with no lock.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use concat_host::export::Exporter;
pub use concat_host::media::{strip_window, window_span, window_start};
use concat_host::playback::{Playback, PlaybackEvents};
use concat_host::preview::Monitor;
use concat_host::{AppDirs, media};
use concat_speech::{Speech, Transcriber};

use crate::gpu::Gpu;
use crate::studio::{Models, Studio};
use crate::ui::App;

/// The engine's long-lived services.
pub struct Host {
    /// Where recents, preferences and models live on this machine.
    pub dirs: AppDirs,
    /// The audio engine and the clock.
    pub playback: Arc<Playback>,
    /// The monitor's reader pool.
    pub monitor: Monitor,
    /// The one-export-at-a-time slot.
    pub exporter: Exporter,
    /// whisper.cpp, and its model downloads.
    pub transcriber: Arc<Transcriber>,
    /// Text to speech, and its model downloads.
    pub speech: Arc<Speech>,
    /// Titles painted to pictures, and the cache of them.
    pub titles: Arc<concat_host::Titles>,
    /// The cutout models, and the masks they find for the project's media.
    pub cutouts: Arc<concat_host::Cutouts>,
    /// The stabilization analysis, and the camera movement it measures.
    pub stabilizers: Arc<concat_host::Stabilizers>,
    /// The brush model, and the regions it reads under smart strokes.
    pub brushes: Arc<concat_host::Brushes>,
    /// The restoration model, and the enhanced copies it writes.
    pub enhancers: Arc<concat_host::Enhancers>,
    pub reversers: Arc<concat_host::Reversers>,
    /// The Concat API on a socket, while the Automation page has it on. Its
    /// own sessions, apart from the window's: a caller edits projects of
    /// its own, never the one on screen, which `open_projects` keeps it
    /// from opening; the export slot is shared, so one export at a time
    /// holds across the two.
    pub server: Option<concat_server::Server>,
    /// Which project folders are open, here or over the socket.
    pub open_projects: concat_api::OpenProjects,
}

impl Host {
    /// Starts every service. The audio device may not be there yet; playback
    /// keeps trying on its own thread and says so through a toast. `gpu` is
    /// the window's device; with it the monitor composites where the window
    /// draws.
    pub fn start(gpu: Option<Gpu>) -> Result<Host, String> {
        let dirs = AppDirs::locate()?;
        let _ = std::fs::create_dir_all(&dirs.config);
        let host = Host {
            titles: Arc::new(concat_host::Titles::new(&dirs)),
            cutouts: Arc::new(concat_host::Cutouts::new(&dirs.data)),
            stabilizers: Arc::new(concat_host::Stabilizers::new()),
            brushes: Arc::new(concat_host::Brushes::new(&dirs.data)),
            enhancers: Arc::new(concat_host::Enhancers::new(&dirs.data)),
            reversers: Arc::new(concat_host::Reversers::new()),
            dirs,
            playback: Playback::start(Arc::new(Events))?,
            monitor: match gpu {
                Some(gpu) => Monitor::with_gpu_watching(gpu.device, gpu.queue, gpu.lost),
                None => Monitor::new(),
            },
            exporter: Exporter::new(),
            transcriber: Arc::new(Transcriber::new()),
            speech: Arc::new(Speech::new()),
            server: None,
            open_projects: concat_api::OpenProjects::default(),
        };
        // Every effect's pipeline, compiled while the window opens rather
        // than at the first frame each effect draws.
        host.monitor.warm_shaders();
        Ok(host)
    }
}

/// Playback's way back to the window. The clock is polled by a timer while
/// playing, so only failures need to cross here.
struct Events;

impl PlaybackEvents for Events {
    fn position(&self, _seconds: f64) {}

    fn error(&self, message: String) {
        let _ = slint::invoke_from_event_loop(move || {
            Shell::with(|shell, app| {
                shell.studio.borrow_mut().notify(&message, true);
                shell.studio.borrow().publish(&app, &shell.models);
            });
        });
    }
}

/// Everything a handler needs: the window, the state and the models the
/// state publishes into. One per process, reachable from the event-loop
/// thread by [`Shell::with`].
pub struct Shell {
    /// The window, weakly: a callback that outlives it is a no-op.
    pub app: slint::Weak<App>,
    /// The window's state.
    pub studio: RefCell<Studio>,
    /// The live models, handed to Slint once and never replaced.
    pub models: Models,
}

thread_local! {
    static SHELL: RefCell<Option<Rc<Shell>>> = const { RefCell::new(None) };
}

impl Shell {
    /// Makes this the process's shell. Called once from `main`.
    pub fn install(shell: Rc<Shell>) {
        SHELL.with(|slot| *slot.borrow_mut() = Some(shell));
    }

    /// Runs `body` with the shell and a strong handle on the window, on the
    /// event-loop thread. Does nothing if the window is gone or the shell
    /// was never installed.
    pub fn with(body: impl FnOnce(&Shell, App)) {
        let shell = SHELL.with(|slot| slot.borrow().clone());
        if let Some(shell) = shell
            && let Some(app) = shell.app.upgrade()
        {
            body(&shell, app);
        }
    }
}

// ── the worker lanes ──
//
// Background work runs on threads kept for the life of the process: a
// thread per job was a thread per monitor frame, thirty a second during
// playback. They come in three lanes, so one kind of work can never sit
// in a queue behind another:
//
//   - the interactive lane: the monitor's frames, the frames ahead of the
//     playhead, and the short jobs the window is waiting on - a probe, a
//     picked colour, a list of fonts ([`spawn`], [`spawn_in_project`],
//     [`spawn_frame`], [`spawn_detached`]);
//   - the long lane: everything that takes seconds to minutes - an export,
//     a merge, captions, speech, cutouts, stabilization, Enhance, reverse,
//     voice isolation, the Smart brush, a download ([`spawn_long`],
//     [`spawn_long_in_project`]);
//   - the save lane: the project's saves, one thread, where a save waiting
//     its turn is replaced by a newer one of the same project
//     ([`spawn_save`]).
//
// These were one queue on three threads, and the comment on it said none
// of the monitor's frame, its prefetch and an analysis ever queued behind
// the others. That held while three things ran. Remove background,
// Stabilize and Captions together took all three: the monitor froze, and
// the autosave queued behind them for minutes, so a crash then lost every
// edit since the last save (audit 2026-11-15, A1). This is the stopgap
// until the engine's own scheduler takes the jobs; the lanes keep the two
// things that must never wait - the picture and the save - off the
// threads the long jobs hold.

/// A job for a lane. `what` is the type of the closure the caller handed
/// in, which names the function that asked for it, for the log.
struct Job {
    what: &'static str,
    run: Box<dyn FnOnce() + Send>,
}

/// A fixed set of threads fed from one queue.
struct Lane {
    sender: std::sync::mpsc::Sender<Job>,
    /// How many threads the lane has.
    threads: usize,
    /// How many of them are running a job now. Read only to say in the log
    /// that a job had to wait, so it can be a moment out of date.
    busy: Arc<std::sync::atomic::AtomicUsize>,
    /// Whether a job that finds every thread busy is said in the log. The
    /// long lane's are: a job waiting there shows no progress, and the log
    /// is where a report of a job "stuck at 0%" is read.
    say_waits: bool,
}

impl Lane {
    /// Starts `threads` threads named `name-0`, `name-1` and so on. A job
    /// that runs longer than `slow` is named in the log as it ends: on the
    /// interactive lane that is a job that belongs on the long one.
    fn start(
        name: &'static str,
        threads: usize,
        slow: Option<std::time::Duration>,
        say_waits: bool,
    ) -> Lane {
        let (sender, receiver) = std::sync::mpsc::channel::<Job>();
        let receiver = Arc::new(std::sync::Mutex::new(receiver));
        let busy = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        for index in 0..threads {
            let receiver = Arc::clone(&receiver);
            let busy = Arc::clone(&busy);
            std::thread::Builder::new()
                .name(format!("{name}-{index}"))
                .spawn(move || {
                    loop {
                        let job = receiver
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .recv();
                        let Ok(job) = job else {
                            return;
                        };
                        busy.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let began = std::time::Instant::now();
                        (job.run)();
                        busy.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                        if let Some(slow) = slow
                            && began.elapsed() > slow
                        {
                            log::warn!(
                                "host: {} held an interactive worker for {}; work that long belongs on the long-job lane, where it cannot hold up the monitor's frames",
                                job.what,
                                concat_log::span(began.elapsed())
                            );
                        }
                    }
                })
                .expect("a worker thread");
        }
        Lane {
            sender,
            threads,
            busy,
            say_waits,
        }
    }

    /// Queues `run` for the lane's next free thread.
    fn submit(&self, what: &'static str, run: Box<dyn FnOnce() + Send>) {
        if self.say_waits && self.busy.load(std::sync::atomic::Ordering::Relaxed) >= self.threads {
            log::info!(
                "host: {what} waits its turn: all {} long-job workers are busy",
                self.threads
            );
        }
        let _ = self.sender.send(Job { what, run });
    }
}

/// The interactive lane's threads: two monitor frames out at once - the
/// one asked for and a refine walking to the exact picture - the prefetch
/// that follows each, and one short job beside them. The monitor never has
/// more in flight than that whatever the machine, so the lane does not
/// grow with it.
const INTERACTIVE_THREADS: usize = 4;

/// A job on the interactive lane that runs longer than this is said in the
/// log. A frame decoded from a keyframe seconds away, or a probe of a large
/// import, is a second or two; past this it is long work on the wrong lane.
const INTERACTIVE_SLOW: std::time::Duration = std::time::Duration::from_secs(5);

/// The long lane's threads on a machine with `cores` hardware threads:
/// half of them, no fewer than three and no more than six. Every long job
/// fans out over the cores on its own - a model's threads, an encoder's,
/// a decoder's - so more of them side by side buys progress on each, not
/// speed, and costs the memory each holds. Three is what a four-thread
/// machine ran before, so it is no more crowded than it was; a bigger one
/// starts a fourth or a fifth job at once rather than holding it at 0%.
fn long_threads(cores: usize) -> usize {
    (cores / 2).clamp(3, 6)
}

fn interactive_lane() -> &'static Lane {
    static LANE: std::sync::OnceLock<Lane> = std::sync::OnceLock::new();
    LANE.get_or_init(|| {
        Lane::start(
            "concat-worker",
            INTERACTIVE_THREADS,
            Some(INTERACTIVE_SLOW),
            false,
        )
    })
}

fn long_lane() -> &'static Lane {
    static LANE: std::sync::OnceLock<Lane> = std::sync::OnceLock::new();
    LANE.get_or_init(|| {
        let cores = std::thread::available_parallelism().map_or(4, |count| count.get());
        let threads = long_threads(cores);
        log::debug!("host: {threads} long-job workers for {cores} hardware threads");
        Lane::start("concat-long", threads, None, true)
    })
}

fn save_lane() -> &'static SaveLane {
    static LANE: std::sync::OnceLock<SaveLane> = std::sync::OnceLock::new();
    LANE.get_or_init(|| SaveLane::start("concat-save"))
}

/// A save waiting on the save lane.
struct SaveJob {
    /// The project it writes; a newer save of the same one replaces it.
    key: String,
    /// Whether the save is to be said when it lands. A Save that is
    /// replaced by an autosave before it runs is still said.
    announce: bool,
    run: Box<dyn FnOnce(bool) + Send>,
}

/// One thread that writes the projects, and the saves waiting for it.
///
/// One thread, so no two saves ever write at once, of one project or of
/// two, and a save never waits behind anything but another save. A save
/// waiting its turn is replaced by a newer one of the same project in its
/// place in the line, so they never pile up behind a slow disk: only the
/// newest snapshot is worth writing, and [`projects::save_ordered`] would
/// skip the older ones anyway. What was asked of a replaced save - to be
/// said - passes to the one that replaces it.
///
/// [`projects::save_ordered`]: concat_host::projects::save_ordered
struct SaveLane {
    shared: Arc<SaveQueue>,
}

#[derive(Default)]
struct SaveQueue {
    waiting: std::sync::Mutex<SaveLine>,
    ready: std::sync::Condvar,
}

#[derive(Default)]
struct SaveLine {
    jobs: std::collections::VecDeque<SaveJob>,
    /// Set when the lane is dropped - only in tests; the process's own
    /// lasts as long as it does - so its thread ends once the line is empty.
    closed: bool,
}

impl SaveLane {
    fn start(name: &'static str) -> SaveLane {
        let shared = Arc::new(SaveQueue::default());
        let queue = Arc::clone(&shared);
        std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                loop {
                    let job = {
                        let mut line = queue
                            .waiting
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        loop {
                            if let Some(job) = line.jobs.pop_front() {
                                break job;
                            }
                            if line.closed {
                                return;
                            }
                            line = queue
                                .ready
                                .wait(line)
                                .unwrap_or_else(|poisoned| poisoned.into_inner());
                        }
                    };
                    (job.run)(job.announce);
                }
            })
            .expect("the save thread");
        SaveLane { shared }
    }

    /// Queues a save of `key`, or puts it in the place of the one of `key`
    /// still waiting.
    fn submit(&self, key: String, announce: bool, run: Box<dyn FnOnce(bool) + Send>) {
        let mut line = self
            .shared
            .waiting
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(waiting) = line.jobs.iter_mut().find(|job| job.key == key) {
            waiting.announce |= announce;
            waiting.run = run;
        } else {
            line.jobs.push_back(SaveJob { key, announce, run });
        }
        self.shared.ready.notify_one();
    }
}

impl Drop for SaveLane {
    fn drop(&mut self) {
        self.shared
            .waiting
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .closed = true;
        self.shared.ready.notify_one();
    }
}

/// Runs `work` on the interactive lane with nothing to report: the frames
/// ahead of the playhead. Never for anything slow; see [`spawn_long`].
pub fn spawn_detached(work: impl FnOnce() + Send + 'static) {
    interactive_lane().submit(std::any::type_name_of_val(&work), Box::new(work));
}

/// Runs `work` on the interactive lane, then `then` on the event-loop
/// thread with the result, the state and the window - followed by a full
/// publish, so a completion never has to remember to redraw.
///
/// For short work the window is waiting on: the lane is the monitor's, and
/// a job that holds one of its threads for seconds holds up the picture.
/// Anything that can take that long goes through [`spawn_long`].
pub fn spawn<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    then: impl FnOnce(&mut Studio, &App, &Models, T) + Send + 'static,
) {
    let what = std::any::type_name_of_val(&work);
    interactive_lane().submit(what, Box::new(move || deliver(None, work(), then)));
}

/// [`spawn`] on the long lane: for work that takes seconds or minutes - a
/// download, a template's copy of its media - and so must not hold a
/// thread the monitor's frames need.
pub fn spawn_long<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    then: impl FnOnce(&mut Studio, &App, &Models, T) + Send + 'static,
) {
    let what = std::any::type_name_of_val(&work);
    long_lane().submit(what, Box::new(move || deliver(None, work(), then)));
}

/// [`spawn_in_project`] on the long lane: an export, a merge, captions,
/// speech, an analysis or a render of the open project's media.
pub fn spawn_long_in_project<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    then: impl FnOnce(&mut Studio, &App, &Models, T) + Send + 'static,
) {
    let epoch = project_epoch();
    let what = std::any::type_name_of_val(&work);
    long_lane().submit(what, Box::new(move || deliver(Some(epoch), work(), then)));
}

/// Runs a save of the project at `key` on the save lane, then `then` with
/// its result and whether it is to be said, and `publish` after - both
/// only while that project is still the open one, like
/// [`spawn_in_project_publishing`].
///
/// A save still waiting when a newer one of the same project comes is
/// replaced by it: its `work` and its `then` are dropped, and the newer
/// one is said if either was to be. `work` must therefore write a
/// snapshot taken when the save was asked for, never read the document
/// when it runs.
pub fn spawn_save<T: Send + 'static>(
    key: String,
    announce: bool,
    work: impl FnOnce() -> T + Send + 'static,
    then: impl FnOnce(&mut Studio, &App, &Models, T, bool) + Send + 'static,
    publish: impl FnOnce(&Studio, &App, &Models) + Send + 'static,
) {
    let epoch = project_epoch();
    save_lane().submit(
        key,
        announce,
        Box::new(move |announce| {
            deliver_then(
                Some(epoch),
                work(),
                move |studio, app, models, result| then(studio, app, models, result, announce),
                publish,
            );
        }),
    );
}

/// The project the window is on, counted up at every open and close. A
/// worker's result made for an earlier project is told apart by it and
/// dropped in [`deliver`], in one place, rather than guarded against in
/// every closure: a probe, a caption run or a spoken line started in one
/// project must not land in the next (audit 2026-09-23, #3).
static PROJECT_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The project epoch now; see [`spawn_in_project`].
pub fn project_epoch() -> u64 {
    PROJECT_EPOCH.load(std::sync::atomic::Ordering::Acquire)
}

/// Starts the next project epoch: a project opened or closed.
pub fn next_project_epoch() {
    PROJECT_EPOCH.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
}

/// Whether a result made in `epoch` concerns a project no longer open.
fn stale(epoch: Option<u64>) -> bool {
    epoch.is_some_and(|epoch| epoch != project_epoch())
}

/// [`spawn`] for work that belongs to the open project: `then` runs only
/// while that project is still the open one, and is dropped otherwise.
/// Work that is not a project's - a poster for the launch screen, a model
/// download - goes through [`spawn`] or [`spawn_long`] and is always
/// delivered. On the interactive lane, like [`spawn`]; long work of the
/// project's goes through [`spawn_long_in_project`].
pub fn spawn_in_project<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    then: impl FnOnce(&mut Studio, &App, &Models, T) + Send + 'static,
) {
    let epoch = project_epoch();
    let what = std::any::type_name_of_val(&work);
    interactive_lane().submit(what, Box::new(move || deliver(Some(epoch), work(), then)));
}

/// [`spawn_in_project`] with the publish after `then` chosen by the caller:
/// for work whose result touches one corner of the window, where a whole
/// publish would be all cost.
pub fn spawn_in_project_publishing<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    then: impl FnOnce(&mut Studio, &App, &Models, T) + Send + 'static,
    publish: impl FnOnce(&Studio, &App, &Models) + Send + 'static,
) {
    let epoch = project_epoch();
    let what = std::any::type_name_of_val(&work);
    interactive_lane().submit(
        what,
        Box::new(move || deliver_then(Some(epoch), work(), then, publish)),
    );
}

/// [`spawn_in_project`] for the monitor's frames. While playback runs a
/// frame arrives every few dozen milliseconds and changes nothing but the
/// picture, so only the picture is published then; the transport's own
/// tick publishes the lanes. Otherwise it is a full publish as usual, and
/// so is a frame that raised a toast - a failed one says so at once, not
/// when playback stops.
pub fn spawn_frame<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    then: impl FnOnce(&mut Studio, &App, &Models, T) + Send + 'static,
) {
    use std::sync::atomic::{AtomicBool, Ordering};
    let epoch = project_epoch();
    let what = std::any::type_name_of_val(&work);
    let run = move || {
        let toasted = Arc::new(AtomicBool::new(false));
        let noted = Arc::clone(&toasted);
        deliver_then(
            Some(epoch),
            work(),
            move |studio, app, models, result| {
                let toast = studio.toast.token;
                then(studio, app, models, result);
                noted.store(studio.toast.token != toast, Ordering::Relaxed);
            },
            move |studio, app, models| {
                if studio.playing && !toasted.load(Ordering::Relaxed) {
                    studio.publish_frame(app);
                } else {
                    studio.publish(app, models);
                }
            },
        );
    };
    interactive_lane().submit(what, Box::new(run));
}

/// Hands a worker's result to the event-loop thread: `then` with the state
/// and the window, and a full publish after it. A result made in a project
/// epoch that has passed is dropped here.
fn deliver<T: Send + 'static>(
    epoch: Option<u64>,
    result: T,
    then: impl FnOnce(&mut Studio, &App, &Models, T) + Send + 'static,
) {
    deliver_then(epoch, result, then, Studio::publish);
}

/// [`deliver`] with the publish after `then` chosen by the caller.
fn deliver_then<T: Send + 'static>(
    epoch: Option<u64>,
    result: T,
    then: impl FnOnce(&mut Studio, &App, &Models, T) + Send + 'static,
    publish: impl FnOnce(&Studio, &App, &Models) + Send + 'static,
) {
    let _ = slint::invoke_from_event_loop(move || {
        Shell::with(|shell, app| {
            if stale(epoch) {
                return;
            }
            {
                let mut studio = shell.studio.borrow_mut();
                then(&mut studio, &app, &shell.models, result);
            }
            publish(&shell.studio.borrow(), &app, &shell.models);
        });
    });
}

/// Like [`spawn`], on the engine's scheduler at the artwork priority:
/// `work` waits its turn behind the monitor's frames, the frames ahead of
/// the playhead and the filmstrips, and only a few artwork jobs ever run
/// together, whatever the size of the import (#52). See
/// `concat_host::scheduler`.
pub fn spawn_art<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    then: impl FnOnce(&mut Studio, &App, &Models, T) + Send + 'static,
) {
    spawn_at(concat_media::Priority::Artwork, work, then);
}

/// Like [`spawn_art`], at the poster priority: a card's picture, ahead
/// of every waveform and filmstrip, and on any free worker.
pub fn spawn_poster<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    then: impl FnOnce(&mut Studio, &App, &Models, T) + Send + 'static,
) {
    spawn_at(concat_media::Priority::Poster, work, then);
}

/// Like [`spawn_art`], at the waveform priority: after the posters and
/// ahead of the files' filmstrips.
pub fn spawn_wave<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    then: impl FnOnce(&mut Studio, &App, &Models, T) + Send + 'static,
) {
    spawn_at(concat_media::Priority::Waveform, work, then);
}

/// Like [`spawn_art`], at the filmstrip priority: the lanes' pictures
/// come before the bin's, after the monitor's.
pub fn spawn_strip<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    then: impl FnOnce(&mut Studio, &App, &Models, T) + Send + 'static,
) {
    spawn_at(concat_media::Priority::Filmstrip, work, then);
}

fn spawn_at<T: Send + 'static>(
    priority: concat_media::Priority,
    work: impl FnOnce() -> T + Send + 'static,
    then: impl FnOnce(&mut Studio, &App, &Models, T) + Send + 'static,
) {
    // Art is of the project open when it was asked for: a strip still
    // decoding when another project opens is dropped, not filed under it.
    let epoch = project_epoch();
    concat_host::scheduler().submit(priority, move || deliver(Some(epoch), work(), then));
}

/// Runs `body` on the event-loop thread from anywhere, with a full publish
/// after. For progress reports from a worker.
pub fn on_ui(body: impl FnOnce(&mut Studio, &App, &Models) + Send + 'static) {
    on_ui_gated(None, body);
}

/// [`on_ui`] for a report about the open project - a job's progress -
/// made in `epoch` (see [`project_epoch`]): dropped once that project has
/// closed, like a [`spawn_in_project`] result.
pub fn on_ui_in_project(
    epoch: u64,
    body: impl FnOnce(&mut Studio, &App, &Models) + Send + 'static,
) {
    on_ui_gated(Some(epoch), body);
}

/// Asks a native dialog on the event-loop thread with the studio
/// unborrowed, and hands its answer to `then` with the studio borrowed,
/// publishing after. For every file and folder dialog.
///
/// A dialog blocks, and on macOS it pumps Slint's timers while it is up;
/// the transport timer and the meters both `borrow_mut` the studio. A
/// dialog opened inside a handler, where the studio is already borrowed
/// for the handler's whole run, was a `BorrowMutError` the moment a timer
/// fired under it - Export › Browse while playing (audit 2026-10-09,
/// finding 6). Here `ask` runs on its own turn of the loop before anything
/// is borrowed, and nothing is published for an answer of `None`: a
/// cancelled dialog changed nothing.
pub fn after_dialog<T: Send + 'static>(
    ask: impl FnOnce() -> Option<T> + Send + 'static,
    then: impl FnOnce(&mut Studio, &App, &Models, T) + Send + 'static,
) {
    let _ = slint::invoke_from_event_loop(move || {
        let Some(answer) = ask() else {
            return;
        };
        Shell::with(|shell, app| {
            {
                let mut studio = shell.studio.borrow_mut();
                then(&mut studio, &app, &shell.models, answer);
            }
            shell.studio.borrow().publish(&app, &shell.models);
        });
    });
}

/// [`after_dialog`] for files, on every platform. The desktop's dialog is
/// asked on a turn of its own with the studio unborrowed. A phone's picker
/// is another screen, which answers later from whichever thread it likes,
/// so it is asked the same way and answered through [`on_ui`]. `then` is
/// not called for a picker dismissed with nothing chosen. `filter` names a
/// family and its extensions, for the desktop's dialog; a phone's offers
/// every kind of media on its own.
pub fn pick_files_then(
    title: String,
    filter: Option<(String, &'static [&'static str])>,
    then: impl FnOnce(&mut Studio, &App, &Models, Vec<std::path::PathBuf>) + Send + 'static,
) {
    if cfg!(any(target_os = "android", target_os = "ios")) {
        let _ = slint::invoke_from_event_loop(move || {
            let filter = filter.as_ref().map(|(name, list)| (name.as_str(), *list));
            crate::platform::pick_files_async(&title, filter, move |paths| {
                if !paths.is_empty() {
                    on_ui(move |studio, app, models| then(studio, app, models, paths));
                }
            });
        });
    } else {
        after_dialog(
            move || {
                let filter = filter.as_ref().map(|(name, list)| (name.as_str(), *list));
                crate::platform::pick_files(&title, filter)
            },
            then,
        );
    }
}

fn on_ui_gated(epoch: Option<u64>, body: impl FnOnce(&mut Studio, &App, &Models) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || {
        Shell::with(|shell, app| {
            if stale(epoch) {
                return;
            }
            {
                let mut studio = shell.studio.borrow_mut();
                body(&mut studio, &app, &shell.models);
            }
            shell.studio.borrow().publish(&app, &shell.models);
        });
    });
}

/// A decoded frame as a Slint image.
pub fn image_of(frame: &concat_core::frame::Frame) -> slint::Image {
    let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
        frame.pixels(),
        frame.width(),
        frame.height(),
    );
    slint::Image::from_rgba8(buffer)
}

/// The picture at `path`, if it can be read, as a Slint image. A file that
/// is not there is `None` without a word: Slint prints every load that
/// fails, and a cached picture not drawn yet is the expected case, not a
/// fault.
pub fn image_at(path: &std::path::Path) -> Option<slint::Image> {
    if !path.is_file() {
        return None;
    }
    slint::Image::load_from_path(path).ok()
}

/// A filmstrip restored from the project's artwork cache.
pub struct CachedStrip {
    /// The strip image.
    pub image: slint::Image,
    /// How many frames the picture holds.
    pub frames: u32,
    /// One frame's width in the picture's pixels.
    pub frame_width: u32,
    /// The picture's height in pixels.
    pub height: u32,
}

/// Artwork restored from the project's cache.
pub struct CachedMediaArt {
    /// Cached thumbnail, when present.
    pub thumbnail: Option<slint::Image>,
    /// Cached filmstrip, when present.
    pub strip: Option<CachedStrip>,
}

/// Loads a media item's thumbnail and filmstrip from the project's JPEG cache.
///
/// The cache key includes the media id plus the source file's size and mtime,
/// so replacing a file with the same project media id naturally misses and
/// regenerates fresh artwork.
pub fn cached_media_art(
    project: &str,
    id: &str,
    path: &str,
    kind: concat_project::model::MediaKind,
) -> CachedMediaArt {
    use concat_project::model::MediaKind;

    if kind == MediaKind::Audio {
        return CachedMediaArt {
            thumbnail: None,
            strip: None,
        };
    }

    let dir = art_cache_dir(project);
    let stem = art_cache_stem(id, path);

    let thumbnail = image_at(&dir.join(format!("{stem}.thumb.jpg")));

    let strip = std::fs::read_dir(&dir).ok().and_then(|entries| {
        let prefix = format!("{stem}.strip.");
        entries.filter_map(Result::ok).find_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            let rest = name.strip_prefix(&prefix)?.strip_suffix(".jpg")?;
            let mut parts = rest.split('.');
            let frames: u32 = parts.next()?.parse().ok()?;
            let frame_width: u32 = parts.next()?.parse().ok()?;
            let height: u32 = parts.next()?.parse().ok()?;
            if parts.next().is_some() {
                return None;
            }
            let image = image_at(&entry.path())?;
            Some(CachedStrip {
                image,
                frames,
                frame_width,
                height,
            })
        })
    });

    CachedMediaArt { thumbnail, strip }
}

fn art_cache_dir(project: &str) -> std::path::PathBuf {
    std::path::Path::new(project).join("cache").join("art")
}

fn art_cache_stem(id: &str, path: &str) -> String {
    format!("{}-{}", clean_cache_part(id), source_stamp(path))
}

fn clean_cache_part(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn source_stamp(path: &str) -> String {
    let Ok(meta) = std::fs::metadata(path) else {
        return "missing".to_owned();
    };
    let len = meta.len();
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    format!("{len:x}-{modified:x}")
}

fn save_media_art_cache(
    project: &str,
    id: &str,
    path: &str,
    thumbnail: Option<&concat_core::frame::Frame>,
    strip: Option<(&concat_core::frame::Frame, u32)>,
) {
    let dir = art_cache_dir(project);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }

    let stem = art_cache_stem(id, path);

    if let Some(frame) = thumbnail {
        let _ = save_frame_jpeg(&dir.join(format!("{stem}.thumb.jpg")), frame, 82);
    }

    if let Some((frame, frames)) = strip {
        let frames = frames.max(1);
        let frame_width = frame.width() / frames;
        let name = format!("{stem}.strip.{frames}.{frame_width}.{}.jpg", frame.height());
        let _ = save_frame_jpeg(&dir.join(name), frame, 78);
    }
}

fn save_frame_jpeg(
    path: &std::path::Path,
    frame: &concat_core::frame::Frame,
    quality: u8,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut rgb = Vec::with_capacity(frame.width() as usize * frame.height() as usize * 3);
    for pixel in frame.pixels().chunks_exact(4) {
        rgb.extend_from_slice(&pixel[..3]);
    }

    let file = std::fs::File::create(path)?;
    let mut writer = std::io::BufWriter::new(file);
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, quality);
    encoder.encode(
        &rgb,
        frame.width(),
        frame.height(),
        image::ColorType::Rgb8.into(),
    )?;
    Ok(())
}

/// A filmstrip of one stretch of a media item, restored from the cache.
///
/// The stretches are cells on a grid over the footage: at `level` k a cell
/// spans 1/2^k of it, and cell `j` starts at j/2^(k+1), so cells overlap by
/// half and any cut no longer than half a cell fits wholly in one. See
/// [`strip_window`].
pub fn cached_window_art(
    project: &str,
    id: &str,
    path: &str,
    level: u32,
    cell: u32,
) -> Option<CachedStrip> {
    let dir = art_cache_dir(project);
    let stem = art_cache_stem(id, path);
    let prefix = format!("{stem}.win.{level}.{cell}.");
    std::fs::read_dir(&dir)
        .ok()?
        .filter_map(Result::ok)
        .find_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            let rest = name.strip_prefix(&prefix)?.strip_suffix(".jpg")?;
            let mut parts = rest.split('.');
            let frames: u32 = parts.next()?.parse().ok()?;
            let frame_width: u32 = parts.next()?.parse().ok()?;
            let height: u32 = parts.next()?.parse().ok()?;
            if parts.next().is_some() {
                return None;
            }
            let image = image_at(&entry.path())?;
            Some(CachedStrip {
                image,
                frames,
                frame_width,
                height,
            })
        })
}

fn save_window_art_cache(
    project: &str,
    id: &str,
    path: &str,
    level: u32,
    cell: u32,
    frame: &concat_core::frame::Frame,
    frames: u32,
) {
    let dir = art_cache_dir(project);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let stem = art_cache_stem(id, path);
    let frames = frames.max(1);
    let frame_width = frame.width() / frames;
    let name = format!(
        "{stem}.win.{level}.{cell}.{frames}.{frame_width}.{}.jpg",
        frame.height()
    );
    let _ = save_frame_jpeg(&dir.join(name), frame, 78);
}

/// The strip of one cell of a media item, decoded on a worker.
pub struct WindowArt {
    /// The media id the strip belongs to.
    pub id: String,
    pub level: u32,
    pub cell: u32,
    /// The frames, side by side, and how many there are.
    pub strip: Option<(concat_core::frame::Frame, u32)>,
}

/// Samples the frames of one cell of a media item - see [`strip_window`] -
/// and writes them to the project's artwork cache beside the file's own.
pub fn window_art(
    id: String,
    path: String,
    project: String,
    level: u32,
    cell: u32,
    duration: f64,
) -> WindowArt {
    let from = window_start(level, cell) * duration;
    let to = from + window_span(level) * duration;
    let strip = media::filmstrip_between(&path, from, to, STRIP_FRAMES, STRIP_HEIGHT)
        .ok()
        .map(|frame| (frame, STRIP_FRAMES));
    if let Some((frame, frames)) = &strip {
        save_window_art_cache(&project, &id, &path, level, cell, frame, *frames);
    }
    WindowArt {
        id,
        level,
        cell,
        strip,
    }
}

/// A probe's error, in the words a toast can use.
pub fn probe_error(error: &str) -> String {
    format!("Could not import: {error}")
}

/// A media item's card picture, decoded on a worker ahead of the rest of
/// its art: what an import is waiting to see. See [`media_poster`].
pub struct MediaPoster {
    /// The media id the picture belongs to.
    pub id: String,
    /// A small frame of the footage or the still. A frame rather than an
    /// image: a Slint image cannot cross a thread, and this is made on one.
    pub thumbnail: Option<concat_core::frame::Frame>,
    /// A still's filmstrip, which is its picture: a strip of one. `None`
    /// for footage, whose strip is sampled across it by [`media_strip`].
    pub strip: Option<(concat_core::frame::Frame, u32)>,
}

/// The waveform or the filmstrip of one media item, decoded on a worker.
pub struct MediaArt {
    /// The media id the art belongs to.
    pub id: String,
    /// Which audio stream the peaks are of: `None` for the file's first,
    /// which is the media's own art, or a named one a clip plays. Named
    /// streams get peaks only, never a picture.
    pub stream: Option<u32>,
    /// The waveform, when it was asked for and the file has sound.
    pub peaks: Option<Arc<concat_media::Pyramid>>,
    /// Frames sampled evenly across the footage, side by side in one
    /// picture, and how many there are, when it was asked for.
    pub strip: Option<(concat_core::frame::Frame, u32)>,
}

/// Frames in a filmstrip, and the strip's height in logical pixels.
///
/// Twenty-four across a file is enough that a clip a few seconds long
/// shows different pictures along its length; the height is the tallest
/// lane's body, so the lanes never upscale it.
const STRIP_FRAMES: u32 = 24;
const STRIP_HEIGHT: u32 = 64;

/// How wide a card's picture is decoded: about twice the card, for a
/// sharp picture on a dense screen.
const POSTER_WIDTH: u32 = 160;

/// Where in the footage a card's picture is taken: a quarter of the way
/// in, and no further than two seconds - past the black leader so many
/// files open on, and near enough the top to be quick to reach. A
/// still's only picture is its first.
fn poster_instant(kind: concat_project::model::MediaKind, duration: Option<f64>) -> f64 {
    if kind == concat_project::model::MediaKind::Image {
        return 0.0;
    }
    duration.map_or(0.0, |seconds| (seconds * 0.25).min(2.0))
}

/// Decodes the card picture for one media item - a single small frame,
/// see `media::card_still` - and files it in the project's artwork cache.
///
/// On its own and first, at the scheduler's poster priority, so a card
/// shows its picture the moment that frame is decoded: not after the
/// file's waveform and filmstrip, and not after every file imported
/// before it has had all of its art made, which on a 4K file is seconds
/// of filmstrip. A still's picture is its filmstrip too.
pub fn media_poster(
    id: String,
    path: String,
    kind: concat_project::model::MediaKind,
    duration: Option<f64>,
    project: String,
) -> MediaPoster {
    use concat_project::model::MediaKind;
    let thumbnail = match kind {
        MediaKind::Video | MediaKind::Image => {
            media::card_still(&path, poster_instant(kind, duration), POSTER_WIDTH).ok()
        }
        MediaKind::Audio => None,
    };
    let strip = (kind == MediaKind::Image)
        .then(|| thumbnail.clone().map(|frame| (frame, 1)))
        .flatten();
    if thumbnail.is_some() {
        save_media_art_cache(
            &project,
            &id,
            &path,
            thumbnail.as_ref(),
            strip.as_ref().map(|(frame, frames)| (frame, *frames)),
        );
    }
    MediaPoster {
        id,
        thumbnail,
        strip,
    }
}

/// Decodes the waveform of one of a media item's audio streams - its
/// first with `stream` `None` - from the project's peaks cache when it
/// is there.
pub fn media_peaks(id: String, path: String, project: String, stream: Option<u32>) -> MediaArt {
    MediaArt {
        peaks: media::peaks(&path, stream, Some(&project))
            .ok()
            .map(|peaks| Arc::new(concat_media::Pyramid::of(peaks))),
        id,
        stream,
        strip: None,
    }
}

/// Decodes the filmstrip of a piece of footage, sampled across the whole
/// of it, and files it in the project's artwork cache.
pub fn media_strip(
    id: String,
    path: String,
    project: String,
    range: Option<concat_media::ColorRange>,
) -> MediaArt {
    // The filmstrip reads the proxy where the file has one: the tiles are
    // small, and a 4K file's frames cost more than they show.
    let strip_source = concat_host::proxy::existing(std::path::Path::new(&project), &path, range)
        .map(|proxy| proxy.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());
    let strip = media::filmstrip(&strip_source, STRIP_FRAMES, STRIP_HEIGHT)
        .ok()
        .map(|frame| (frame, STRIP_FRAMES));
    if let Some((frame, frames)) = &strip {
        save_media_art_cache(&project, &id, &path, None, Some((frame, *frames)));
    }
    MediaArt {
        id,
        stream: None,
        peaks: None,
        strip,
    }
}

#[cfg(test)]
mod epoch_tests {
    #[test]
    fn a_result_from_an_earlier_project_is_stale() {
        let then = super::project_epoch();
        assert!(
            !super::stale(None),
            "work that is nobody's project is never stale"
        );
        assert!(!super::stale(Some(then)));
        super::next_project_epoch();
        assert!(
            super::stale(Some(then)),
            "the project it was made in has closed"
        );
        assert!(!super::stale(Some(super::project_epoch())));
    }
}

#[cfg(test)]
mod lane_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// What "at once" is in these tests: far more than a job on an idle
    /// thread takes to start, far less than the minutes a save waited.
    const PROMPTLY: Duration = Duration::from_secs(1);

    #[test]
    fn the_long_lane_grows_with_the_machine_within_bounds() {
        assert_eq!(super::long_threads(1), 3, "never fewer than three");
        assert_eq!(super::long_threads(4), 3, "a four-thread machine as before");
        assert_eq!(super::long_threads(8), 4);
        assert_eq!(super::long_threads(12), 6);
        assert_eq!(super::long_threads(64), 6, "never more than six");
    }

    /// The bug: every long job running took a worker the monitor's frames
    /// and the autosave needed. With the long lane full - and a job more
    /// waiting behind it - a frame and a save still run at once.
    #[test]
    fn a_frame_and_a_save_run_at_once_with_the_long_lane_full() {
        let long = super::long_lane();
        let (release, held) = mpsc::channel::<()>();
        let held = Arc::new(Mutex::new(held));
        let started = Arc::new(AtomicUsize::new(0));
        let blockers = long.threads + 2;
        for _ in 0..blockers {
            let held = Arc::clone(&held);
            let started = Arc::clone(&started);
            long.submit(
                "a long job that does not end",
                Box::new(move || {
                    started.fetch_add(1, Ordering::SeqCst);
                    let _ = held.lock().expect("the gate").recv();
                }),
            );
        }
        let waited = Instant::now();
        while started.load(Ordering::SeqCst) < long.threads {
            assert!(
                waited.elapsed() < Duration::from_secs(10),
                "the long lane never filled"
            );
            std::thread::yield_now();
        }
        assert_eq!(
            started.load(Ordering::SeqCst),
            long.threads,
            "every long worker is held and the jobs after them wait"
        );

        let asked = Instant::now();
        let (frame_done, frame) = mpsc::channel();
        super::interactive_lane().submit(
            "a monitor frame",
            Box::new(move || {
                let _ = frame_done.send(());
            }),
        );
        let (save_done, save) = mpsc::channel();
        super::save_lane().submit(
            "lane-test-project".to_owned(),
            false,
            Box::new(move |_| {
                let _ = save_done.send(());
            }),
        );
        let frame = frame.recv_timeout(PROMPTLY);
        let save = save.recv_timeout(PROMPTLY);
        let took = asked.elapsed();

        // Let the long jobs go before asserting, so a failure here does not
        // leave the lane held for the tests after it.
        for _ in 0..blockers {
            let _ = release.send(());
        }
        assert!(frame.is_ok(), "the frame waited behind the long jobs");
        assert!(save.is_ok(), "the save waited behind the long jobs");
        assert!(took < PROMPTLY, "both ran in {took:?}");
    }

    /// One run of a save job: which project, which of its snapshots, and
    /// whether it was to be said.
    type Ran = (String, u32, bool);

    /// A save job that records itself and how many saves were running
    /// beside it.
    fn recording(
        key: &str,
        number: u32,
        ran: &Arc<Mutex<Vec<Ran>>>,
        running: &Arc<AtomicUsize>,
        most: &Arc<AtomicUsize>,
        hold: Duration,
    ) -> Box<dyn FnOnce(bool) + Send> {
        let (key, ran, running, most) = (
            key.to_owned(),
            Arc::clone(ran),
            Arc::clone(running),
            Arc::clone(most),
        );
        Box::new(move |announce| {
            let now = running.fetch_add(1, Ordering::SeqCst) + 1;
            most.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(hold);
            ran.lock()
                .expect("the record")
                .push((key, number, announce));
            running.fetch_sub(1, Ordering::SeqCst);
        })
    }

    /// Saves asked for while one is writing do not pile up: the waiting
    /// one of a project is replaced by the newest, in its place in the
    /// line, a Save replaced by an autosave is still said, and another
    /// project's save keeps its own turn.
    #[test]
    fn a_waiting_save_is_replaced_by_the_newest_of_its_project() {
        let lane = super::SaveLane::start("concat-save-test");
        let ran = Arc::new(Mutex::new(Vec::new()));
        let (running, most) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));

        // The first save holds the thread until it is let go, so the ones
        // after it are all waiting together.
        let (writing, wrote) = mpsc::channel::<()>();
        let (release, gate) = mpsc::channel::<()>();
        let first = recording("a", 1, &ran, &running, &most, Duration::ZERO);
        lane.submit(
            "a".to_owned(),
            false,
            Box::new(move |announce| {
                let _ = writing.send(());
                let _ = gate.recv();
                first(announce);
            }),
        );
        wrote.recv_timeout(PROMPTLY).expect("the first save starts");

        let job = |key: &str, number| recording(key, number, &ran, &running, &most, Duration::ZERO);
        lane.submit("a".to_owned(), true, job("a", 2));
        lane.submit("b".to_owned(), false, job("b", 1));
        for number in 3..=6 {
            lane.submit("a".to_owned(), false, job("a", number));
        }
        let _ = release.send(());

        let waited = Instant::now();
        while ran.lock().expect("the record").len() < 3 && waited.elapsed() < PROMPTLY * 5 {
            std::thread::sleep(Duration::from_millis(2));
        }
        // Long enough for a save that should not run to have run.
        std::thread::sleep(Duration::from_millis(50));
        let ran = ran.lock().expect("the record").clone();
        assert_eq!(
            ran,
            vec![
                ("a".to_owned(), 1, false),
                ("a".to_owned(), 6, true),
                ("b".to_owned(), 1, false),
            ],
            "the newest of a, in the place of the first waiting, and said; b in its turn"
        );
        assert_eq!(most.load(Ordering::SeqCst), 1, "never two saves at once");
    }

    /// Saves asked for far faster than they are written: never two at
    /// once, each project's in the order they were asked for, the last of
    /// each written, and fewer writes than asks.
    #[test]
    fn saves_asked_for_faster_than_they_write_keep_the_newest() {
        let lane = super::SaveLane::start("concat-save-stress");
        let ran = Arc::new(Mutex::new(Vec::new()));
        let (running, most) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let keys = ["a", "b", "c"];
        let asks = 300u32;
        for number in 1..=asks {
            let key = keys[number as usize % keys.len()];
            lane.submit(
                key.to_owned(),
                false,
                recording(key, number, &ran, &running, &most, Duration::from_millis(1)),
            );
        }
        let last: Vec<u32> = keys
            .iter()
            .map(|key| {
                (1..=asks)
                    .filter(|number| keys[*number as usize % keys.len()] == *key)
                    .max()
                    .expect("every key is asked for")
            })
            .collect();

        let waited = Instant::now();
        loop {
            let done = {
                let ran = ran.lock().expect("the record");
                keys.iter().zip(&last).all(|(key, newest)| {
                    ran.iter()
                        .any(|(ran_key, number, _)| ran_key == key && number == newest)
                })
            };
            if done {
                break;
            }
            assert!(
                waited.elapsed() < Duration::from_secs(10),
                "the newest save of every project lands"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(50));
        let ran = ran.lock().expect("the record").clone();
        assert_eq!(most.load(Ordering::SeqCst), 1, "never two saves at once");
        for (key, newest) in keys.iter().zip(&last) {
            let numbers: Vec<u32> = ran
                .iter()
                .filter(|(ran_key, _, _)| ran_key == key)
                .map(|(_, number, _)| *number)
                .collect();
            assert!(
                numbers.windows(2).all(|pair| pair[0] < pair[1]),
                "{key}'s saves ran in the order they were asked for: {numbers:?}"
            );
            assert_eq!(
                numbers.last(),
                Some(newest),
                "{key}'s newest is the last written"
            );
        }
        assert!(
            ran.len() < asks as usize,
            "the waiting saves were replaced, not each written: {} of {asks}",
            ran.len()
        );
    }
}
