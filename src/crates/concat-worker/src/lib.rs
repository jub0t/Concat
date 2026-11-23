// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Risky work in a process of its own.
//!
//! The editor runs a great deal of code it does not own: the graphics
//! driver under every frame, FFmpeg's decoders, a chip's encoder, whisper.cpp
//! and ONNX Runtime. Any of it can crash, hang or abort, and in the editor's
//! process it took the editor with it: an export that lost its device took
//! the preview's too (Direct3D 12 keeps one device per adapter in a
//! process), and a transcription that overran its stack closed the window
//! (#673, #736). This crate is the one way such work leaves the editor:
//!
//! - **One trait.** A [`Job`] says what it is asked, what it reports and
//!   what it hands back, all as serde types, and how long it may be quiet.
//! - **One protocol.** The editor starts its own program again with
//!   `--worker <name>`, writes the request as one line of JSON, and reads
//!   one JSON object a line back ([`Line`]). `cancel` on a line stops the
//!   job; standard input closing stops it too, so a worker whose editor is
//!   gone does not run on. The worker's log goes to standard error, which
//!   the editor copies into its own.
//! - **One supervisor.** [`run`] starts the worker, reads it, cancels it,
//!   kills one that will not stop or has gone quiet past what its job
//!   allows, and tries again as the job's [`Policy`] says - by default once
//!   more with drawing on the CPU, where no driver is in the way.
//! - **The same job in-process.** Where no process can be started - a
//!   phone, a sandbox - [`run_here`] runs the job on the calling thread
//!   with the same context, so a job is written once.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub use concat_core::Halt;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// The flag a program is started with to be a worker; the job's name
/// follows it.
pub const FLAG: &str = "--worker";

/// The version of [`Line`]s this build speaks, said first.
const PROTOCOL: u32 = 1;

/// How often a worker says it is alive, from a thread of its own: a worker
/// stuck in a driver still says so, and one that stops saying so is frozen
/// or gone.
const HEARTBEAT: Duration = Duration::from_secs(2);

/// How long a worker may go without a heartbeat before it is taken for
/// frozen.
const NO_PULSE: Duration = Duration::from_secs(30);

/// Work that may crash, hang or abort, run where that costs only itself.
pub trait Job {
    /// The name after [`FLAG`], and in the log.
    const NAME: &'static str;
    /// What the job is asked.
    type Request: Serialize + DeserializeOwned;
    /// What it says as it goes.
    type Progress: Serialize + DeserializeOwned;
    /// What it hands back.
    type Output: Serialize + DeserializeOwned;

    /// The work, in whichever process runs it.
    fn run(request: Self::Request, context: &Context<Self::Progress>)
    -> Result<Self::Output, Halt>;

    /// How long the job may go without reporting progress before its
    /// worker is taken for hung, after this progress was the last it said
    /// (`None` before any). A job that reports every frame wants minutes;
    /// one whose one step is a long mix wants more.
    fn quiet(_last: Option<&Self::Progress>) -> Duration {
        Duration::from_secs(5 * 60)
    }
}

/// What a running job has of its caller: a way to report, and a way to be
/// told to stop.
pub struct Context<P> {
    cancel: Arc<AtomicBool>,
    report: Arc<dyn Fn(&P) + Send + Sync>,
}

impl<P> Context<P> {
    /// Says how the job is going.
    pub fn progress(&self, progress: &P) {
        (self.report)(progress);
    }

    /// Whether the job has been told to stop.
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// The flag behind [`Context::cancelled`], for engine code that takes
    /// one.
    pub fn cancel_flag(&self) -> &AtomicBool {
        &self.cancel
    }

    /// The same flag, owned, for a thread that outlives this borrow.
    pub fn cancel_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    /// [`Context::progress`] as an owned function, for engine code that
    /// wants a reporter it can keep: a callback that must be `'static`.
    pub fn reporter(&self) -> Arc<dyn Fn(&P) + Send + Sync> {
        Arc::clone(&self.report)
    }
}

/// One line between a worker and its editor, as JSON.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Line<P, O> {
    /// The worker is up and speaks this version.
    Hello(u32),
    /// The worker is alive, whatever its job is doing.
    Alive,
    /// The job's progress.
    Progress(P),
    /// The job finished with this.
    Done(O),
    /// The job failed, and this is why.
    Failed(String),
    /// The job stopped when told to.
    Cancelled,
}

/// What a worker could be started as: the name and the function that runs
/// it in the worker. Built with [`entry`]; the program's worker table is a
/// list of them, and [`serve`] picks one by name.
pub struct Entry {
    /// The job's name.
    pub name: &'static str,
    serve: fn() -> i32,
}

/// The [`Entry`] for `J`.
pub fn entry<J: Job + 'static>() -> Entry {
    Entry {
        name: J::NAME,
        serve: serve_job::<J>,
    }
}

/// The worker's side: runs the job `name` names from `table`, reading its
/// request on standard input and answering on standard output, and hands
/// back the exit code. The program calls this when started with [`FLAG`],
/// before it opens a window, and exits with what it returns.
pub fn serve(table: &[Entry], name: &str) -> i32 {
    match table.iter().find(|entry| entry.name == name) {
        Some(entry) => (entry.serve)(),
        None => {
            let mut out = std::io::stdout().lock();
            let _ = writeln!(
                out,
                "{}",
                serde_json::json!({ "failed": format!("no job is called {name:?}") })
            );
            2
        }
    }
}

/// One line on standard output, whole and flushed. Several threads speak
/// - the job, the heartbeat - so each line is written under the lock.
fn say<P: Serialize, O: Serialize>(line: &Line<P, O>) {
    let Ok(text) = serde_json::to_string(line) else {
        return;
    };
    let mut out = std::io::stdout().lock();
    // An editor that has gone leaves nobody to tell.
    let _ = writeln!(out, "{text}");
    let _ = out.flush();
}

fn serve_job<J: Job>() -> i32 {
    say::<(), ()>(&Line::Hello(PROTOCOL));
    let _ = std::thread::Builder::new()
        .name("concat worker pulse".into())
        .spawn(|| {
            loop {
                say::<(), ()>(&Line::Alive);
                std::thread::sleep(HEARTBEAT);
            }
        });
    let mut first = String::new();
    if std::io::stdin().lock().read_line(&mut first).is_err() || first.trim().is_empty() {
        say::<(), ()>(&Line::Failed("the worker was given no request".into()));
        return 1;
    }
    let request: J::Request = match serde_json::from_str(&first) {
        Ok(request) => request,
        Err(error) => {
            say::<(), ()>(&Line::Failed(format!("the request did not read: {error}")));
            return 1;
        }
    };
    let cancel = Arc::new(AtomicBool::new(false));
    {
        // `cancel`, or the editor gone: either way, stop.
        let cancel = Arc::clone(&cancel);
        let _ = std::thread::Builder::new()
            .name("concat worker orders".into())
            .spawn(move || {
                for line in std::io::stdin().lock().lines() {
                    match line {
                        Ok(line) if line.trim() == "cancel" => break,
                        Ok(_) => continue,
                        Err(_) => break,
                    }
                }
                cancel.store(true, Ordering::Relaxed);
            });
    }
    let context = Context {
        cancel,
        report: Arc::new(|progress: &J::Progress| {
            say::<&J::Progress, ()>(&Line::Progress(progress));
        }),
    };
    match J::run(request, &context) {
        Ok(output) => {
            say::<(), J::Output>(&Line::Done(output));
            0
        }
        Err(Halt::Cancelled) => {
            say::<(), ()>(&Line::Cancelled);
            0
        }
        Err(Halt::Failed(why)) => {
            say::<(), ()>(&Line::Failed(why));
            1
        }
    }
}

/// How the editor runs a job: the tries it makes, each with the variables
/// it sets, and how long a worker told to stop may go on.
#[derive(Clone, Debug)]
pub struct Policy {
    /// The tries, in order; the next is made only when a worker died. The
    /// default is two: as the machine is, then with drawing on the CPU.
    pub tries: Vec<Try>,
    /// How long a worker told to stop may go on before it is killed.
    pub stop_grace: Duration,
}

/// One try of a job: what it is called in the log, and the environment
/// variables its worker is started with.
#[derive(Clone, Debug)]
pub struct Try {
    /// Said in the log and to the caller: "on the GPU", "on the CPU".
    pub label: &'static str,
    /// Set for this worker alone.
    pub env: Vec<(&'static str, &'static str)>,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            tries: vec![
                Try {
                    label: "as the machine is",
                    env: Vec::new(),
                },
                // A safe start's setting: the compositor draws on the
                // software adapter, and no graphics driver is in the way.
                Try {
                    label: "drawing on the CPU",
                    env: vec![("CONCAT_GPU", "software")],
                },
            ],
            stop_grace: Duration::from_secs(15),
        }
    }
}

impl Policy {
    /// One try, as the machine is: for a job a second try would not help.
    pub fn once() -> Policy {
        Policy {
            tries: Policy::default().tries.into_iter().take(1).collect(),
            ..Policy::default()
        }
    }
}

/// What the caller of [`run`] hears as the job goes.
#[derive(Debug)]
pub enum Event<P> {
    /// The job's own progress.
    Progress(P),
    /// A worker died, and the job is being tried again: why it died, and
    /// the label of the try now starting.
    Retrying {
        /// How the last worker died.
        why: String,
        /// The [`Try::label`] of the next.
        next: &'static str,
    },
}

/// How one worker's run ended.
#[derive(Debug)]
pub enum Ended<P, O> {
    /// It said how the job went.
    Said(Result<O, Halt>),
    /// It stopped without saying: crashed, killed, frozen or hung. Why, and
    /// the last progress it reported.
    Died {
        /// Exit status or the watchdog's reason, and where it had got.
        why: String,
        /// The last progress it reported.
        last: Option<P>,
    },
}

/// Runs `J` on `request` in workers of `program`, as `policy` says, and
/// hands back how it went. `None` when no worker could be started at all,
/// and the caller then runs the job with [`run_here`].
pub fn run<J: Job>(
    program: &Path,
    request: &J::Request,
    policy: &Policy,
    cancel: &AtomicBool,
    hear: impl FnMut(Event<J::Progress>),
) -> Option<Result<J::Output, Halt>> {
    run_with::<J>(
        &|| worker_command::<J>(program),
        request,
        policy,
        cancel,
        hear,
    )
}

/// The command that starts `program` as `J`'s worker.
fn worker_command<J: Job>(program: &Path) -> Command {
    let mut command = Command::new(program);
    command.arg(FLAG).arg(J::NAME);
    command
}

/// [`run`] with the command that starts a worker given: what the tests
/// start this crate's own test binary with.
fn run_with<J: Job>(
    make: &dyn Fn() -> Command,
    request: &J::Request,
    policy: &Policy,
    cancel: &AtomicBool,
    mut hear: impl FnMut(Event<J::Progress>),
) -> Option<Result<J::Output, Halt>> {
    let line = match serde_json::to_string(request) {
        Ok(line) => line,
        Err(error) => {
            return Some(Err(Halt::Failed(format!(
                "the {} job could not be handed over: {error}",
                J::NAME
            ))));
        }
    };
    let mut deaths: Vec<String> = Vec::new();
    for (index, attempt) in policy.tries.iter().enumerate() {
        if index > 0 {
            let why = deaths.last().cloned().unwrap_or_default();
            log::error!(
                "worker: the {} job's process stopped without finishing ({why}); trying it again {}",
                J::NAME,
                attempt.label
            );
            hear(Event::Retrying {
                why,
                next: attempt.label,
            });
        }
        let ended = once_with::<J>(make, &line, attempt, policy, cancel, &mut |progress| {
            hear(Event::Progress(progress));
        });
        match ended {
            None if index == 0 => return None,
            None => deaths.push("no process could be started".to_owned()),
            Some(Ended::Said(result)) => return Some(result),
            Some(Ended::Died { why, .. }) => {
                if cancel.load(Ordering::Relaxed) {
                    return Some(Err(Halt::Cancelled));
                }
                deaths.push(format!("{}: {why}", attempt.label));
            }
        }
    }
    Some(Err(Halt::Failed(format!(
        "the {} job stopped each time it was tried: {}",
        J::NAME,
        deaths.join("; ")
    ))))
}

/// One worker of `program` for `J`, started with `attempt`'s variables,
/// handed `request` (already JSON) and read to its end. `None` when it
/// could not be started. For a caller with its own idea of what to try
/// next - the encoder probe, which goes on without the encoder a worker
/// died opening.
pub fn once<J: Job>(
    program: &Path,
    request: &str,
    attempt: &Try,
    policy: &Policy,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(J::Progress),
) -> Option<Ended<J::Progress, J::Output>> {
    once_with::<J>(
        &|| worker_command::<J>(program),
        request,
        attempt,
        policy,
        cancel,
        progress,
    )
}

/// [`once`] with the command that starts the worker given.
fn once_with<J: Job>(
    make: &dyn Fn() -> Command,
    request: &str,
    attempt: &Try,
    policy: &Policy,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(J::Progress),
) -> Option<Ended<J::Progress, J::Output>> {
    let mut command = make();
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in &attempt.env {
        command.env(key, value);
    }
    #[cfg(windows)]
    {
        // No console window of its own over the editor's.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            log::warn!(
                "worker: no process could be started for the {} job ({error})",
                J::NAME
            );
            return None;
        }
    };
    let (Some(mut stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        let _ = child.kill();
        return None;
    };
    let name = J::NAME;
    // The worker's log, into the editor's, line by line as it comes, at the
    // level the worker wrote it. Under this crate's own target: the log
    // writes another crate's lines only from a warning up, and a worker's
    // whole account of an export is information.
    let logger = std::thread::Builder::new()
        .name("concat worker log".into())
        .spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                log::log!(level_of(&line), "[{name}] {line}");
            }
        })
        .ok();
    // Its lines, onto a channel, so the watchdog runs while it is quiet.
    let (lines_tx, lines) = std::sync::mpsc::channel::<String>();
    let reader = std::thread::Builder::new()
        .name("concat worker answers".into())
        .spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if lines_tx.send(line).is_err() {
                    break;
                }
            }
        })
        .ok();

    let handed = writeln!(stdin, "{request}").and_then(|()| stdin.flush());
    let mut last: Option<J::Progress> = None;
    let mut greeted = false;
    let mut told_to_stop: Option<Instant> = None;
    let mut pulse = Instant::now();
    let mut heard = Instant::now();
    let kill = |child: &mut std::process::Child| {
        let _ = child.kill();
    };
    let ended = loop {
        if handed.is_err() {
            break Ended::Died {
                why: "it would not take the request".to_owned(),
                last: None,
            };
        }
        if cancel.load(Ordering::Relaxed) && told_to_stop.is_none() {
            let _ = writeln!(stdin, "cancel").and_then(|()| stdin.flush());
            told_to_stop = Some(Instant::now());
        }
        if told_to_stop.is_some_and(|when| when.elapsed() > policy.stop_grace) {
            kill(&mut child);
            break Ended::Said(Err(Halt::Cancelled));
        }
        if pulse.elapsed() > NO_PULSE {
            kill(&mut child);
            break Ended::Died {
                why: format!("it stopped answering for {}s", NO_PULSE.as_secs()),
                last,
            };
        }
        let quiet = J::quiet(last.as_ref());
        if heard.elapsed() > quiet {
            kill(&mut child);
            break Ended::Died {
                why: format!("it made no progress for {}s", quiet.as_secs()),
                last,
            };
        }
        match lines.recv_timeout(Duration::from_millis(100)) {
            Ok(text) => {
                pulse = Instant::now();
                let line: Line<J::Progress, J::Output> = match serde_json::from_str(&text) {
                    Ok(line) => line,
                    Err(_) => {
                        // Not ours: a library that printed to standard
                        // output. Kept in the log, not taken as an answer.
                        log::info!("[{name}] said {text}");
                        continue;
                    }
                };
                match line {
                    Line::Hello(version) => {
                        greeted = true;
                        if version != PROTOCOL {
                            kill(&mut child);
                            break Ended::Said(Err(Halt::Failed(format!(
                                "the worker speaks version {version}, this build {PROTOCOL}"
                            ))));
                        }
                    }
                    Line::Alive => {}
                    Line::Progress(report) => {
                        heard = Instant::now();
                        progress_and_keep::<J>(report, &mut last, progress);
                    }
                    Line::Done(output) => break Ended::Said(Ok(output)),
                    Line::Failed(why) => break Ended::Said(Err(Halt::Failed(why))),
                    Line::Cancelled => break Ended::Said(Err(Halt::Cancelled)),
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                let status = child
                    .wait()
                    .map_or_else(|error| error.to_string(), |status| status.to_string());
                let at = if greeted {
                    "after it started"
                } else {
                    "before it spoke"
                };
                break Ended::Died {
                    why: format!("{status}, {at}"),
                    last,
                };
            }
        }
    };
    drop(stdin);
    let _ = child.wait();
    if let Some(reader) = reader {
        let _ = reader.join();
    }
    if let Some(logger) = logger {
        let _ = logger.join();
    }
    Some(ended)
}

/// The level a worker wrote one line of its log at, read from the line: the
/// log writes the level as a word between spaces. A line with none - a
/// library printing to standard error - is information.
fn level_of(line: &str) -> log::Level {
    let words = line.split_whitespace().take(3);
    for word in words {
        match word {
            "ERROR" => return log::Level::Error,
            "WARN" => return log::Level::Warn,
            "DEBUG" => return log::Level::Debug,
            "TRACE" => return log::Level::Trace,
            "INFO" => return log::Level::Info,
            _ => {}
        }
    }
    log::Level::Info
}

/// Hands `report` to the caller and keeps it as the last progress: the
/// caller's copy is made by a round trip through JSON, which every
/// progress type has, so no type needs `Clone`.
fn progress_and_keep<J: Job>(
    report: J::Progress,
    last: &mut Option<J::Progress>,
    progress: &mut dyn FnMut(J::Progress),
) {
    let copy = serde_json::to_string(&report)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok());
    progress(report);
    *last = copy;
}

/// Runs `J` on the calling thread, with the same context a worker gives
/// it: for where no worker can be started, and for tests.
pub fn run_here<J: Job>(
    request: J::Request,
    cancel: Arc<AtomicBool>,
    report: impl Fn(&J::Progress) + Send + Sync + 'static,
) -> Result<J::Output, Halt> {
    let context = Context {
        cancel,
        report: Arc::new(report),
    };
    J::run(request, &context)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_line_reads_back_as_it_was_written() {
        let lines: Vec<Line<u32, String>> = vec![
            Line::Hello(PROTOCOL),
            Line::Alive,
            Line::Progress(7),
            Line::Done("written".into()),
            Line::Failed("the encoder\nwould not open".into()),
            Line::Cancelled,
        ];
        for line in lines {
            let text = serde_json::to_string(&line).expect("a line writes");
            assert!(!text.contains('\n'), "one line: {text}");
            let back: Line<u32, String> = serde_json::from_str(&text).expect("a line reads");
            assert_eq!(back, line);
        }
    }

    struct Count;

    impl Job for Count {
        const NAME: &'static str = "count";
        type Request = u32;
        type Progress = u32;
        type Output = u32;

        fn run(request: u32, context: &Context<u32>) -> Result<u32, Halt> {
            for step in 0..request {
                if context.cancelled() {
                    return Err(Halt::Cancelled);
                }
                context.progress(&step);
            }
            Ok(request * 2)
        }
    }

    #[test]
    fn a_worker_line_keeps_the_level_it_was_written_at() {
        assert_eq!(
            level_of("03:55:47.299 ERROR gpu     the device was lost"),
            log::Level::Error
        );
        assert_eq!(
            level_of("03:55:47.299 WARN  export  slow"),
            log::Level::Warn
        );
        assert_eq!(
            level_of("03:55:47.299 INFO  export  done"),
            log::Level::Info
        );
        assert_eq!(
            level_of("a library said ERROR somewhere late"),
            log::Level::Info
        );
    }

    #[test]
    fn a_job_runs_here_with_the_context_a_worker_gives_it() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let into = Arc::clone(&seen);
        let out = run_here::<Count>(3, Arc::new(AtomicBool::new(false)), move |step| {
            into.lock().expect("seen").push(*step);
        });
        assert_eq!(out, Ok(6));
        assert_eq!(*seen.lock().expect("seen"), vec![0, 1, 2]);
        let stopped = run_here::<Count>(3, Arc::new(AtomicBool::new(true)), |_| {});
        assert_eq!(stopped, Err(Halt::Cancelled));
    }

    #[test]
    fn the_default_policy_tries_again_on_the_cpu() {
        let policy = Policy::default();
        assert_eq!(policy.tries.len(), 2);
        assert!(policy.tries[0].env.is_empty());
        assert_eq!(policy.tries[1].env, vec![("CONCAT_GPU", "software")]);
        assert_eq!(Policy::once().tries.len(), 1);
    }

    #[test]
    fn a_program_that_is_not_there_starts_no_worker() {
        let gone = Path::new("this-program-is-not-there-anywhere.exe");
        let out = run::<Count>(
            gone,
            &3,
            &Policy::default(),
            &AtomicBool::new(false),
            |_| {},
        );
        assert!(out.is_none());
    }

    /// How the stand-in worker below is told to behave.
    const PLAY: &str = "CONCAT_WORKER_TEST_PLAY";

    /// The stand-in's job: counts to its request, and does what [`PLAY`]
    /// says when it reaches 2 - unless drawing "on the CPU", the second
    /// try, where it always counts to the end.
    struct Play;

    impl Job for Play {
        const NAME: &'static str = "play";
        type Request = u32;
        type Progress = u32;
        type Output = String;

        fn run(request: u32, context: &Context<u32>) -> Result<String, Halt> {
            let play = std::env::var(PLAY).unwrap_or_default();
            let second = std::env::var("CONCAT_GPU").is_ok_and(|gpu| gpu == "software");
            for step in 0..request {
                if step == 2 && !second {
                    match play.as_str() {
                        "crash" => std::process::abort(),
                        "hang" => std::thread::sleep(Duration::from_secs(600)),
                        "deaf" => {
                            // Told to stop, and never looks.
                            std::thread::sleep(Duration::from_secs(600));
                        }
                        "fail" => return Err(Halt::Failed("the encoder would not open".into())),
                        _ => {}
                    }
                }
                if context.cancelled() {
                    return Err(Halt::Cancelled);
                }
                context.progress(&step);
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(format!(
                "counted to {request}{}",
                if second { " on the CPU" } else { "" }
            ))
        }

        fn quiet(_last: Option<&u32>) -> Duration {
            Duration::from_secs(3)
        }
    }

    /// Not a test of anything: the worker the tests below start, which is
    /// this test binary run again for this one test with [`PLAY`] set.
    #[test]
    fn stand_in_worker() {
        if std::env::var_os(PLAY).is_none() {
            return;
        }
        std::process::exit(serve_job::<Play>());
    }

    /// Runs the stand-in as `play`, counting to 5; what was heard and how
    /// it ended.
    fn played(
        play: &'static str,
        cancel_after: Option<Duration>,
    ) -> (Vec<String>, Option<Result<String, Halt>>) {
        let program = std::env::current_exe().expect("the test binary");
        let make = move || {
            let mut command = Command::new(&program);
            command
                .args(["--exact", "tests::stand_in_worker", "--nocapture"])
                .env(PLAY, play);
            command
        };
        let policy = Policy {
            stop_grace: Duration::from_secs(2),
            ..Policy::default()
        };
        let cancel = Arc::new(AtomicBool::new(false));
        if let Some(after) = cancel_after {
            let cancel = Arc::clone(&cancel);
            std::thread::spawn(move || {
                std::thread::sleep(after);
                cancel.store(true, Ordering::Relaxed);
            });
        }
        let mut heard = Vec::new();
        let ended = run_with::<Play>(&make, &5, &policy, &cancel, |event| {
            heard.push(match event {
                Event::Progress(step) => step.to_string(),
                Event::Retrying { next, .. } => format!("retry {next}"),
            });
        });
        (heard, ended)
    }

    #[test]
    fn a_worker_that_finishes_hands_back_its_output() {
        let (heard, ended) = played("well", None);
        assert_eq!(ended, Some(Ok("counted to 5".to_owned())));
        assert_eq!(heard, ["0", "1", "2", "3", "4"]);
    }

    /// The driver under the job crashes its process: the editor - this
    /// test - is still here, and the second try, on the CPU, finishes.
    #[test]
    fn a_worker_that_crashes_is_tried_again_on_the_cpu() {
        let (heard, ended) = played("crash", None);
        assert_eq!(ended, Some(Ok("counted to 5 on the CPU".to_owned())));
        assert!(
            heard.contains(&"retry drawing on the CPU".to_owned()),
            "{heard:?}"
        );
    }

    #[test]
    fn a_worker_that_hangs_is_killed_and_tried_again() {
        let (heard, ended) = played("hang", None);
        assert_eq!(ended, Some(Ok("counted to 5 on the CPU".to_owned())));
        assert!(
            heard.iter().any(|line| line.starts_with("retry")),
            "{heard:?}"
        );
    }

    /// A job that fails says why, and is not tried again: a second try
    /// would fail the same way.
    #[test]
    fn a_job_that_fails_is_not_tried_again() {
        let (heard, ended) = played("fail", None);
        assert_eq!(
            ended,
            Some(Err(Halt::Failed("the encoder would not open".to_owned())))
        );
        assert!(
            !heard.iter().any(|line| line.starts_with("retry")),
            "{heard:?}"
        );
    }

    /// A worker stuck where it never looks at its cancel is killed once
    /// the grace has passed, and the job ends cancelled, not tried again.
    #[test]
    fn a_worker_that_will_not_stop_is_killed_after_its_grace() {
        let began = Instant::now();
        let (heard, ended) = played("deaf", Some(Duration::from_millis(400)));
        assert_eq!(ended, Some(Err(Halt::Cancelled)));
        assert!(
            !heard.iter().any(|line| line.starts_with("retry")),
            "{heard:?}"
        );
        assert!(
            began.elapsed() < Duration::from_secs(20),
            "{:?}",
            began.elapsed()
        );
    }
}
