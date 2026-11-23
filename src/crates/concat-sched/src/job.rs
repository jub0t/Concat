// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A job: what it runs, what it carries, and how it ends.

use std::any::Any;
use std::convert::Infallible;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::budget::Lane;
use crate::cancel::{CancelReason, CancelToken, Cancelled};
use crate::lock;
use crate::memory::Kept;
use crate::scheduler::Core;

/// A job's number: unique for the life of its scheduler, and in the order
/// the jobs were handed in.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct JobId(pub(crate) u64);

impl JobId {
    /// The number itself.
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Who a job belongs to: an open project, say. The scheduler does not
/// look inside; it is there so that everything one owner has can be
/// cancelled together when the owner goes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Owner(pub u64);

/// What makes two jobs the same job as far as running both goes: "the
/// save of this project", "the frame on the monitor", "the export". What
/// happens when a second arrives is its [`Coalesce`].
///
/// One namespace across every lane, so the text should say what kind of
/// job it is as well as which.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct JobKey(Arc<str>);

impl JobKey {
    /// A key from its text.
    pub fn new(text: impl Into<Arc<str>>) -> JobKey {
        JobKey(text.into())
    }

    /// The key's text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for JobKey {
    fn from(text: &str) -> JobKey {
        JobKey::new(text)
    }
}

impl From<String> for JobKey {
    fn from(text: String) -> JobKey {
        JobKey::new(text)
    }
}

impl fmt::Display for JobKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a job was run on, exactly enough that the same key means the
/// same result: the file and its size and date, and the version of what
/// reads it. A failure is remembered under it; see
/// [`Job::input`](crate::Job::input).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct InputKey(Arc<str>);

impl InputKey {
    /// A key from its text.
    pub fn new(text: impl Into<Arc<str>>) -> InputKey {
        InputKey(text.into())
    }

    /// The key for one file as one kind of job reads it: `what` names the
    /// job and the version of its method ("cutout 3"), and the rest is
    /// the file as it stood - its path, its length and when it was last
    /// written. Edit the file or change the method and the key is another
    /// key, so the job is tried afresh.
    ///
    /// Nothing is read here; the caller has the figures from the probe.
    pub fn for_file(what: &str, path: &Path, bytes: u64, modified: Option<SystemTime>) -> InputKey {
        let modified = modified
            .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |since| since.as_nanos());
        InputKey::new(format!(
            "{what}|{}|{bytes}|{modified}",
            path.to_string_lossy()
        ))
    }

    /// The key's text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for InputKey {
    fn from(text: &str) -> InputKey {
        InputKey::new(text)
    }
}

impl From<String> for InputKey {
    fn from(text: String) -> InputKey {
        InputKey::new(text)
    }
}

impl fmt::Display for InputKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Something there is only so much of, counted: bytes of graphics
/// memory, bytes of memory for models, encoder sessions. A job says how
/// much it [needs](crate::Job::needs), is not started until that much is
/// free, and gives it back when it ends, however it ends.
///
/// How much there is, is the host's to say
/// ([`Scheduler::set_capacity`](crate::Scheduler::set_capacity)); until
/// it has, there is none.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Resource(&'static str);

impl Resource {
    /// Graphics memory, in bytes.
    pub const GPU_MEMORY: Resource = Resource("gpu-memory");
    /// Memory held by loaded models, in bytes.
    pub const MODEL_MEMORY: Resource = Resource("model-memory");
    /// Hardware encoder sessions, of which a graphics card allows a few.
    pub const ENCODER_SESSIONS: Resource = Resource("encoder-sessions");

    /// A resource by name. Two with the same name are the same resource.
    pub const fn new(name: &'static str) -> Resource {
        Resource(name)
    }

    /// The resource's name.
    pub const fn name(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for Resource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// What happens when a job arrives and one with the same [`JobKey`] is
/// still waiting or running.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Coalesce {
    /// The newest wins. One still waiting is dropped, and the new job
    /// takes its place in the line; one running is cancelled, and the new
    /// job is not started until it has stopped, so two with one key never
    /// run at once. A save and the frame on screen: only the latest is
    /// worth having, and two saves must never write one file together.
    Replace,
    /// The first wins. The new job is refused, with the number of the one
    /// in its way. An export: a second press does not start a second.
    KeepFirst,
    /// Both run, and the key is only a name to cancel them by.
    RunBoth,
}

/// Why a job stopped short, as the job itself says it.
///
/// `?` on a [`JobContext::checkpoint`] gives the first; the second is the
/// job's own error, which never has to carry a cancel inside it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum JobError<E> {
    /// The job saw that it was to stop, and did.
    Cancelled,
    /// The job failed.
    Failed(E),
}

impl<E> From<Cancelled> for JobError<E> {
    fn from(_: Cancelled) -> JobError<E> {
        JobError::Cancelled
    }
}

/// Turns a step's own error into the job's, so `?` can be used on it
/// beside the `?` on a checkpoint.
pub trait OrFail<T, E> {
    /// `Err(error)` as `Err(JobError::Failed(error))`.
    fn or_fail(self) -> Result<T, JobError<E>>;
}

impl<T, E> OrFail<T, E> for Result<T, E> {
    fn or_fail(self) -> Result<T, JobError<E>> {
        self.map_err(JobError::Failed)
    }
}

/// Why a job was not taken at all.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Refusal {
    /// Its lane already has as many waiting as it holds. Nothing is
    /// queued without a bound; see
    /// [`Config::queue_limit`](crate::Config::queue_limit).
    QueueFull,
    /// The scheduler has been shut down.
    ShutDown,
    /// A job with the same key is waiting or running, and this one asked
    /// to [keep the first](Coalesce::KeepFirst). The number is that job's.
    Duplicate(JobId),
    /// It needs more of a resource than there is in all, so waiting would
    /// never end.
    TooLarge(Resource),
    /// None of its lane's threads could be started.
    NoWorker,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::QueueFull => f.write_str("its lane's queue is full"),
            Refusal::ShutDown => f.write_str("the scheduler is shut down"),
            Refusal::Duplicate(first) => write!(f, "job {first} with the same key is in its way"),
            Refusal::TooLarge(resource) => write!(f, "it needs more {resource} than there is"),
            Refusal::NoWorker => f.write_str("its lane has no thread"),
        }
    }
}

/// How a job ended, with what it made or what went wrong.
///
/// A cancel is its own ending and never a kind of failure. It also wins:
/// a job whose token was cancelled before it ended is `Cancelled` whatever
/// it returned, so what a cancelled job made is never taken for a result,
/// and code that answers a cancel with an error of its own needs no
/// translating.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome<T, E> {
    /// It ran to the end.
    Done(T),
    /// It ran and failed.
    Failed(E),
    /// It was not run: its [input](crate::Job::input) failed before, with
    /// this error, and nothing has changed since. Not news - whoever
    /// showed the first failure need not show this one.
    FailedBefore(E),
    /// It was asked to stop, before it started or while it ran.
    Cancelled(CancelReason),
    /// It panicked; the text is the panic's message. The thread it ran on
    /// and the scheduler carry on.
    Panicked(String),
    /// It was not taken.
    Refused(Refusal),
}

impl<T, E> Outcome<T, E> {
    /// The ending without what it carries.
    pub fn end(&self) -> End {
        match self {
            Outcome::Done(_) => End::Done,
            Outcome::Failed(_) => End::Failed,
            Outcome::FailedBefore(_) => End::FailedBefore,
            Outcome::Cancelled(reason) => End::Cancelled(*reason),
            Outcome::Panicked(message) => End::Panicked(message.clone()),
            Outcome::Refused(refusal) => End::Refused(*refusal),
        }
    }

    /// What it made, if it ran to the end.
    pub fn done(self) -> Option<T> {
        match self {
            Outcome::Done(value) => Some(value),
            _ => None,
        }
    }

    /// Whether it was cancelled, for whatever reason.
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Outcome::Cancelled(_))
    }
}

/// How a job ended, as an [`Outcome`] says it but without the value or
/// the error: what an observer is told.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum End {
    /// It ran to the end.
    Done,
    /// It ran and failed.
    Failed,
    /// It was not run, because its input failed before.
    FailedBefore,
    /// It was asked to stop.
    Cancelled(CancelReason),
    /// It panicked, with this message.
    Panicked(String),
    /// It was not taken.
    Refused(Refusal),
}

/// Where a job is.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum JobState {
    /// Waiting: for a thread of its lane, for cores, for a resource, or
    /// for the job it replaces to stop.
    Queued,
    /// Running.
    Running,
    /// Standing aside at a checkpoint for a lane above; see
    /// [`JobContext::checkpoint`].
    Paused,
    /// Asked to stop and not yet stopped.
    Stopping,
    /// Over.
    Ended,
}

impl JobState {
    const fn code(self) -> u8 {
        match self {
            JobState::Queued => 0,
            JobState::Running => 1,
            JobState::Paused => 2,
            JobState::Stopping => 3,
            JobState::Ended => 4,
        }
    }

    const fn from_code(code: u8) -> JobState {
        match code {
            0 => JobState::Queued,
            1 => JobState::Running,
            2 => JobState::Paused,
            3 => JobState::Stopping,
            _ => JobState::Ended,
        }
    }
}

/// How far a job has got.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Progress {
    /// From nothing, `0.0`, to all of it, `1.0`.
    pub fraction: f32,
    /// The key of the label for what it is doing now ("export.mixing"),
    /// if it has said. A key and not the words, which are the window's to
    /// find in the person's language.
    pub stage: Option<&'static str>,
}

/// One job as an observer sees it at one moment.
#[derive(Clone, PartialEq, Debug)]
pub struct JobInfo {
    /// Its number.
    pub id: JobId,
    /// Its lane.
    pub lane: Lane,
    /// What kind of job it is, for the log: "export", "cutout".
    pub label: &'static str,
    /// Who it belongs to.
    pub owner: Option<Owner>,
    /// Its key, if it has one.
    pub key: Option<JobKey>,
    /// Where it is.
    pub state: JobState,
    /// How far it has got.
    pub progress: Progress,
    /// The cores it was granted for its own threads; nothing until it
    /// starts.
    pub threads: usize,
    /// How long ago it was handed in.
    pub age: Duration,
    /// How long ago it started, if it has.
    pub running_for: Option<Duration>,
}

/// The part of a job that everyone with a hand on it shares: the worker
/// running it, the handle waiting for it, the scheduler's own lists.
pub(crate) struct JobShared {
    pub(crate) id: JobId,
    pub(crate) lane: Lane,
    pub(crate) label: &'static str,
    pub(crate) owner: Option<Owner>,
    pub(crate) key: Option<JobKey>,
    pub(crate) token: CancelToken,
    /// The fraction done, as the bits of an `f32`.
    progress: AtomicU32,
    /// Behind a lock of its own, which only a change of stage and a look
    /// at it take; the fraction, which a job writes in its inner loop,
    /// never does.
    stage: Mutex<Option<&'static str>>,
    /// Set by the scheduler while a lane above wants the cores this job
    /// holds; read at each checkpoint.
    pub(crate) aside: AtomicBool,
    /// The cores granted when it started.
    pub(crate) threads: AtomicUsize,
    state: AtomicU8,
    submitted: Instant,
    started: OnceLock<Instant>,
    ended: Mutex<Option<End>>,
    ended_changed: Condvar,
}

impl JobShared {
    pub(crate) fn new(
        id: JobId,
        lane: Lane,
        label: &'static str,
        owner: Option<Owner>,
        key: Option<JobKey>,
    ) -> JobShared {
        JobShared {
            id,
            lane,
            label,
            owner,
            key,
            token: CancelToken::new(),
            progress: AtomicU32::new(0f32.to_bits()),
            stage: Mutex::new(None),
            aside: AtomicBool::new(false),
            threads: AtomicUsize::new(0),
            state: AtomicU8::new(JobState::Queued.code()),
            submitted: Instant::now(),
            started: OnceLock::new(),
            ended: Mutex::new(None),
            ended_changed: Condvar::new(),
        }
    }

    pub(crate) fn set_state(&self, state: JobState) {
        self.state.store(state.code(), Ordering::Release);
    }

    /// Where the job is; a job running or paused whose token has been
    /// cancelled is stopping.
    pub(crate) fn state(&self) -> JobState {
        let state = JobState::from_code(self.state.load(Ordering::Acquire));
        match state {
            JobState::Running | JobState::Paused if self.token.is_cancelled() => JobState::Stopping,
            other => other,
        }
    }

    /// Notes that the job has been given to a thread, with `threads` cores.
    pub(crate) fn started(&self, threads: usize) {
        self.threads.store(threads, Ordering::Release);
        let _ = self.started.set(Instant::now());
        self.set_state(JobState::Running);
    }

    /// How long the job has been running, if it has started.
    pub(crate) fn running_for(&self) -> Option<Duration> {
        self.started.get().map(Instant::elapsed)
    }

    pub(crate) fn progress(&self) -> Progress {
        Progress {
            fraction: f32::from_bits(self.progress.load(Ordering::Relaxed)),
            stage: *lock(&self.stage),
        }
    }

    pub(crate) fn info(&self) -> JobInfo {
        JobInfo {
            id: self.id,
            lane: self.lane,
            label: self.label,
            owner: self.owner,
            key: self.key.clone(),
            state: self.state(),
            progress: self.progress(),
            threads: self.threads.load(Ordering::Acquire),
            age: self.submitted.elapsed(),
            running_for: self.running_for(),
        }
    }

    /// Marks the job over, and wakes whoever waits for that. The last
    /// thing that happens to a job: its outcome has been handed on and
    /// its event sent by now.
    pub(crate) fn finish(&self, end: End) {
        self.token.retire();
        self.set_state(JobState::Ended);
        let mut ended = lock(&self.ended);
        *ended = Some(end);
        self.ended_changed.notify_all();
    }

    fn end(&self) -> Option<End> {
        lock(&self.ended).clone()
    }

    /// Blocks until the job is over or `deadline` comes; `None` waits for
    /// as long as it takes.
    pub(crate) fn wait_until(&self, deadline: Option<Instant>) -> Option<End> {
        let mut ended = lock(&self.ended);
        loop {
            if let Some(end) = ended.as_ref() {
                return Some(end.clone());
            }
            ended = match deadline {
                None => self
                    .ended_changed
                    .wait(ended)
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
                Some(deadline) => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return None;
                    }
                    self.ended_changed
                        .wait_timeout(ended, left)
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .0
                }
            };
        }
    }
}

/// A hand on a job that does not need to know what the job makes: to
/// cancel it, watch it, or wait for it to be over. Cheap to clone.
///
/// Dropping it does nothing to the job.
#[derive(Clone)]
pub struct Ticket {
    pub(crate) job: Arc<JobShared>,
}

impl Ticket {
    /// The job's number.
    pub fn id(&self) -> JobId {
        self.job.id
    }

    /// The job's lane.
    pub fn lane(&self) -> Lane {
        self.job.lane
    }

    /// The job's token, to hand to whatever may cancel it.
    pub fn token(&self) -> &CancelToken {
        &self.job.token
    }

    /// Asks the job to stop. One still waiting is over by the time this
    /// returns, and never runs; one running stops at its next check.
    /// Returns whether this call was the one that asked.
    pub fn cancel(&self) -> bool {
        self.job.token.cancel()
    }

    /// How far the job has got, and what it says it is doing. The
    /// scheduler's lock is not taken; the stage is behind a small lock of
    /// the job's own, which the job takes only when its stage changes.
    pub fn progress(&self) -> Progress {
        self.job.progress()
    }

    /// The fraction of [`Ticket::progress`] alone: one atomic load and no
    /// lock of any kind, for a bar redrawn every frame.
    pub fn fraction(&self) -> f32 {
        f32::from_bits(self.job.progress.load(Ordering::Relaxed))
    }

    /// Where the job is. No lock is taken.
    pub fn state(&self) -> JobState {
        self.job.state()
    }

    /// The job as it is now.
    pub fn info(&self) -> JobInfo {
        self.job.info()
    }

    /// Whether the job is over: ended, its outcome handed on, everything
    /// it held given back.
    pub fn is_finished(&self) -> bool {
        self.job.end().is_some()
    }

    /// How the job ended, if it has.
    pub fn end(&self) -> Option<End> {
        self.job.end()
    }

    /// Blocks until the job is over or `timeout` has passed. `None` if it
    /// is still going.
    pub fn wait(&self, timeout: Duration) -> Option<End> {
        match Instant::now().checked_add(timeout) {
            Some(deadline) => self.job.wait_until(Some(deadline)),
            None => self.job.wait_until(None),
        }
    }
}

impl fmt::Debug for Ticket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ticket")
            .field("id", &self.job.id)
            .field("lane", &self.job.lane)
            .field("label", &self.job.label)
            .field("state", &self.job.state())
            .finish()
    }
}

/// Where a job's outcome is put for its handle to take.
type Slot<T, E> = Arc<Mutex<Option<Outcome<T, E>>>>;

/// A hand on a job and on what it makes.
///
/// Everything a [`Ticket`] does, and the [`Outcome`] when the job is
/// over. Dropping it does not cancel the job; the job runs on and its
/// outcome is dropped.
pub struct JobHandle<T, E = Infallible> {
    ticket: Ticket,
    slot: Slot<T, E>,
}

impl<T, E> JobHandle<T, E> {
    pub(crate) fn new(ticket: Ticket, slot: Slot<T, E>) -> JobHandle<T, E> {
        JobHandle { ticket, slot }
    }

    /// The untyped hand on the same job, to keep or to pass on.
    pub fn ticket(&self) -> Ticket {
        self.ticket.clone()
    }

    /// Blocks until the job is over and returns how it ended.
    pub fn wait(self) -> Outcome<T, E> {
        self.ticket.job.wait_until(None);
        self.take()
    }

    /// Blocks until the job is over or `timeout` has passed. A job still
    /// going gives the handle back.
    pub fn wait_timeout(self, timeout: Duration) -> Result<Outcome<T, E>, JobHandle<T, E>> {
        if self.ticket.wait(timeout).is_some() {
            Ok(self.take())
        } else {
            Err(self)
        }
    }

    fn take(self) -> Outcome<T, E> {
        let outcome = lock(&self.slot).take();
        // The outcome is put in its place before the job is marked over,
        // so it is there; the other arm is for a fault in the scheduler.
        outcome.unwrap_or_else(|| Outcome::Panicked("the scheduler lost this job's outcome".into()))
    }
}

impl<T, E> std::ops::Deref for JobHandle<T, E> {
    type Target = Ticket;

    fn deref(&self) -> &Ticket {
        &self.ticket
    }
}

impl<T, E> fmt::Debug for JobHandle<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("JobHandle").field(&self.ticket).finish()
    }
}

/// What a running job is handed: its token, its progress, and the cores
/// it may use.
pub struct JobContext {
    pub(crate) job: Arc<JobShared>,
    pub(crate) core: Arc<Core>,
}

impl JobContext {
    /// The job's number.
    pub fn id(&self) -> JobId {
        self.job.id
    }

    /// The job's lane.
    pub fn lane(&self) -> Lane {
        self.job.lane
    }

    /// Who the job belongs to.
    pub fn owner(&self) -> Option<Owner> {
        self.job.owner
    }

    /// The job's token, to hand down to whatever does the work.
    pub fn token(&self) -> &CancelToken {
        &self.job.token
    }

    /// Whether the job is to stop. One atomic load.
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        self.job.token.is_cancelled()
    }

    /// The call for the top of a job's loop: `Err(Cancelled)` if the job
    /// is to stop, and otherwise a place the scheduler may hold it.
    ///
    /// A job on a lane that shares the cores can be running on cores a
    /// lane above then wants. The job above is started regardless and
    /// never waits; this one, if at least half its cores are wanted, is
    /// asked to stand aside, and the next time it comes here it does: it
    /// gives its cores back and blocks until they are free again, or
    /// until it is cancelled. Nothing it holds in memory, and no
    /// resource, is given up meanwhile.
    ///
    /// Two atomic loads when there is nothing to do, so it can be called
    /// per frame or per block. A job that never comes here cannot be held
    /// and keeps its cores to the end; it should not ask for many.
    #[inline]
    pub fn checkpoint(&self) -> Result<(), Cancelled> {
        self.job.token.check()?;
        if self.job.aside.load(Ordering::Relaxed) {
            self.core.stand_aside(&self.job)?;
        }
        Ok(())
    }

    /// Says how far the job has got, from `0.0` to `1.0`. One atomic
    /// store, so it can be said as often as the job likes.
    #[inline]
    pub fn progress(&self, fraction: f32) {
        if fraction.is_nan() {
            return;
        }
        self.job
            .progress
            .store(fraction.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// Says what the job is doing now, as the key of a label.
    pub fn stage(&self, key: &'static str) {
        *lock(&self.job.stage) = Some(key);
    }

    /// The cores this job was granted, and so the most threads it should
    /// start for itself: what it hands to its encoder, its decoder or its
    /// model in place of a cap of its own. At least one.
    pub fn threads(&self) -> usize {
        self.job.threads.load(Ordering::Acquire).max(1)
    }
}

/// The work of a job, with its types.
type Work<T, E> = Box<dyn FnOnce(&JobContext) -> Result<T, JobError<E>> + Send>;

/// A job's own error, into and back out of what the failure memory keeps.
pub(crate) struct Remembering<E> {
    keep: fn(&E) -> Kept,
    recall: fn(&(dyn Any + Send + Sync)) -> Option<E>,
}

/// A job to hand to a [`Scheduler`](crate::Scheduler): the work, and
/// everything said about it.
///
/// ```
/// use concat_sched::{Coalesce, Job, JobError, Lane, OrFail, Owner};
///
/// fn frames() -> Result<u32, String> {
///     Ok(240)
/// }
///
/// let job = Job::new(Lane::Background, |ctx| {
///     let frames = frames().or_fail()?;
///     for frame in 0..frames {
///         ctx.checkpoint()?;
///         // ... one frame's work ...
///         ctx.progress(frame as f32 / frames as f32);
///     }
///     Ok::<_, JobError<String>>(frames)
/// })
/// .label("stabilize")
/// .owner(Owner(7))
/// .key("stabilize clip-12", Coalesce::Replace)
/// .threads(4);
/// # let _ = job;
/// ```
///
/// One thing a job must not do is wait, inside its work, for another job
/// to end. While it waits it holds its thread and its cores, and if the
/// other job is on a lane below, those may be the very cores the other
/// is standing aside from: neither would move. Work that follows other
/// work is handed in when the first is over, from the callback of
/// [`Scheduler::submit_then`](crate::Scheduler::submit_then).
pub struct Job<T, E = Infallible> {
    pub(crate) lane: Lane,
    pub(crate) label: &'static str,
    pub(crate) owner: Option<Owner>,
    pub(crate) key: Option<(JobKey, Coalesce)>,
    pub(crate) input: Option<InputKey>,
    pub(crate) needs: Vec<(Resource, u64)>,
    pub(crate) rank: u8,
    pub(crate) most: usize,
    pub(crate) least: usize,
    work: Work<T, E>,
    remembering: Option<Remembering<E>>,
}

impl<T: Send + 'static, E: Send + 'static> Job<T, E> {
    /// A job for `lane` that can be cancelled part-way and can fail.
    ///
    /// `work` checks [`JobContext::checkpoint`] as it goes, with `?`, and
    /// turns its own errors with [`OrFail::or_fail`].
    pub fn new<F>(lane: Lane, work: F) -> Job<T, E>
    where
        F: FnOnce(&JobContext) -> Result<T, JobError<E>> + Send + 'static,
    {
        Job {
            lane,
            label: std::any::type_name::<F>(),
            owner: None,
            key: None,
            input: None,
            needs: Vec::new(),
            rank: 0,
            most: 1,
            least: 1,
            work: Box::new(work),
            remembering: None,
        }
    }

    /// A job for `lane` whose work returns a plain `Result`.
    ///
    /// For work that hears of a cancel some way of its own - a flag it
    /// polls ([`CancelToken::as_flag`]) - and answers with an error of
    /// its own: because a cancel wins, that error is never mistaken for a
    /// failure.
    pub fn fallible<F>(lane: Lane, work: F) -> Job<T, E>
    where
        F: FnOnce(&JobContext) -> Result<T, E> + Send + 'static,
    {
        let mut job = Job::new(lane, move |ctx: &JobContext| {
            work(ctx).map_err(JobError::Failed)
        });
        job.label = std::any::type_name::<F>();
        job
    }

    /// What kind of job this is, for the log and the snapshot. Without
    /// one it is the type name of the work, which names the function that
    /// asked for it.
    pub fn label(mut self, label: &'static str) -> Job<T, E> {
        self.label = label;
        self
    }

    /// Who the job belongs to; see
    /// [`Scheduler::cancel_owner`](crate::Scheduler::cancel_owner).
    pub fn owner(mut self, owner: Owner) -> Job<T, E> {
        self.owner = Some(owner);
        self
    }

    /// The job's key, and what is done when another job has it already.
    pub fn key(mut self, key: impl Into<JobKey>, coalesce: Coalesce) -> Job<T, E> {
        self.key = Some((key.into(), coalesce));
        self
    }

    /// The job's place among those waiting on its lane: a lower rank
    /// goes first, and equal ranks go in the order they came. Nothing
    /// without one.
    pub fn rank(mut self, rank: u8) -> Job<T, E> {
        self.rank = rank;
        self
    }

    /// The most threads the job can put to use for itself; it is granted
    /// up to that many cores ([`JobContext::threads`]). One without this.
    /// Counts only on a lane that [shares the cores](Lane::shares_cores).
    pub fn threads(mut self, most: usize) -> Job<T, E> {
        self.most = most.max(1);
        self.least = self.least.min(self.most);
        self
    }

    /// The fewest cores the job is worth starting with: it waits until
    /// that many are free to it, or as many as its lane can ever have.
    /// For work that fixes its threads when it starts, which would
    /// otherwise begin on one core beside an export and stay on one for
    /// the hour after.
    pub fn at_least(mut self, least: usize) -> Job<T, E> {
        self.least = least.max(1);
        self.most = self.most.max(self.least);
        self
    }

    /// As many cores as the lane will grant.
    pub fn wide(self) -> Job<T, E> {
        self.threads(usize::MAX)
    }

    /// `amount` of `resource`, held from before the job starts until it
    /// has ended - by finishing, failing, a cancel or a panic. Asked for
    /// twice, the amounts add up.
    pub fn needs(mut self, resource: Resource, amount: u64) -> Job<T, E> {
        if amount == 0 {
            return self;
        }
        match self.needs.iter_mut().find(|(held, _)| *held == resource) {
            Some((_, held)) => *held = held.saturating_add(amount),
            None => self.needs.push((resource, amount)),
        }
        self
    }
}

impl<T: Send + 'static, E: Clone + Send + Sync + 'static> Job<T, E> {
    /// What the job is run on. If it fails, the failure is remembered
    /// under this key, and a later job with the same key is not run: it
    /// ends at once as [`Outcome::FailedBefore`] with the same error,
    /// until the key is forgotten
    /// ([`Scheduler::forget`](crate::Scheduler::forget)) or the input,
    /// and with it the key, changes.
    ///
    /// Only a failure is remembered. A cancel says nothing about the
    /// input, and a panic is a fault in the job.
    pub fn input(mut self, key: impl Into<InputKey>) -> Job<T, E> {
        self.input = Some(key.into());
        self.remembering = Some(Remembering {
            keep: |error| Arc::new(error.clone()),
            recall: |kept| kept.downcast_ref::<E>().cloned(),
        });
        self
    }
}

impl<T: Send + 'static> Job<T, Infallible> {
    /// A job for `lane` that cannot fail and stops only by returning.
    ///
    /// It can still be cancelled: it looks at
    /// [`JobContext::is_cancelled`] and returns early with whatever it
    /// has, and because a cancel wins, that is never taken for a result.
    pub fn infallible<F>(lane: Lane, work: F) -> Job<T, Infallible>
    where
        F: FnOnce(&JobContext) -> T + Send + 'static,
    {
        let mut job = Job::new(lane, move |ctx: &JobContext| Ok(work(ctx)));
        job.label = std::any::type_name::<F>();
        job
    }
}

/// Where a job's outcome goes.
pub(crate) enum Sink<T, E> {
    /// Into the handle's slot.
    Slot(Slot<T, E>),
    /// To the callback given with the job.
    Then(Box<dyn FnOnce(Outcome<T, E>) + Send>),
}

/// A job that ended without being run.
pub(crate) enum Never {
    /// Cancelled while it waited.
    Cancelled(CancelReason),
    /// Not taken.
    Refused(Refusal),
}

/// How a run ended, as much of it as the scheduler needs.
pub(crate) struct Ending {
    pub(crate) end: End,
    /// The job's error, to be remembered under its input.
    pub(crate) kept: Option<Kept>,
}

/// A job with its types put away, so jobs of every kind wait in one queue.
pub(crate) trait Task: Send {
    /// Runs the work and keeps its outcome.
    fn run(&mut self, ctx: &JobContext) -> Ending;
    /// Ends the job unrun with the failure its input is remembered for.
    /// `false`, and nothing done, if `kept` is not this job's kind of
    /// error: another kind of job used the same key.
    fn recall(&mut self, kept: &(dyn Any + Send + Sync)) -> bool;
    /// Ends the job unrun.
    fn settle(&mut self, never: Never) -> End;
    /// Hands the outcome on: into the handle's slot or to the callback.
    fn deliver(self: Box<Self>);
}

pub(crate) struct Typed<T, E> {
    work: Option<Work<T, E>>,
    remembering: Option<Remembering<E>>,
    outcome: Option<Outcome<T, E>>,
    sink: Sink<T, E>,
}

impl<T: Send + 'static, E: Send + 'static> Typed<T, E> {
    /// Takes the work out of `job`; what is left is what the scheduler
    /// reads.
    pub(crate) fn boxed(job: Job<T, E>, sink: Sink<T, E>) -> (Box<dyn Task>, Said) {
        let said = Said {
            lane: job.lane,
            label: job.label,
            owner: job.owner,
            key: job.key,
            input: job.input,
            needs: job.needs,
            rank: job.rank,
            most: job.most,
            least: job.least,
        };
        let task = Typed {
            work: Some(job.work),
            remembering: job.remembering,
            outcome: None,
            sink,
        };
        (Box::new(task), said)
    }
}

/// Everything said about a job, apart from its work.
pub(crate) struct Said {
    pub(crate) lane: Lane,
    pub(crate) label: &'static str,
    pub(crate) owner: Option<Owner>,
    pub(crate) key: Option<(JobKey, Coalesce)>,
    pub(crate) input: Option<InputKey>,
    pub(crate) needs: Vec<(Resource, u64)>,
    pub(crate) rank: u8,
    pub(crate) most: usize,
    pub(crate) least: usize,
}

impl<T: Send + 'static, E: Send + 'static> Task for Typed<T, E> {
    fn run(&mut self, ctx: &JobContext) -> Ending {
        let Some(work) = self.work.take() else {
            // Run twice, which the scheduler never does; the first
            // outcome stands.
            return Ending {
                end: self.outcome.as_ref().map_or(End::Done, Outcome::end),
                kept: None,
            };
        };
        // The work is dropped inside the call, with whatever it captured,
        // so a panic in a captured value's drop is caught with the rest.
        let result = catch_unwind(AssertUnwindSafe(move || work(ctx)));
        // Read once, here: a cancel that comes after this is too late to
        // change how the job ended, and one that came before decides it.
        let cancelled = ctx.job.token.reason();
        let mut kept = None;
        let outcome = match (result, cancelled) {
            (Err(panic), _) => Outcome::Panicked(panic_message(panic.as_ref())),
            (Ok(_), Some(reason)) => Outcome::Cancelled(reason),
            (Ok(Ok(value)), None) => Outcome::Done(value),
            (Ok(Err(JobError::Cancelled)), None) => Outcome::Cancelled(CancelReason::Requested),
            (Ok(Err(JobError::Failed(error))), None) => {
                kept = self
                    .remembering
                    .as_ref()
                    .map(|remembering| (remembering.keep)(&error));
                Outcome::Failed(error)
            }
        };
        let end = outcome.end();
        self.outcome = Some(outcome);
        Ending { end, kept }
    }

    fn recall(&mut self, kept: &(dyn Any + Send + Sync)) -> bool {
        let Some(error) = self
            .remembering
            .as_ref()
            .and_then(|remembering| (remembering.recall)(kept))
        else {
            return false;
        };
        self.work = None;
        self.outcome = Some(Outcome::FailedBefore(error));
        true
    }

    fn settle(&mut self, never: Never) -> End {
        self.work = None;
        let outcome = match never {
            Never::Cancelled(reason) => Outcome::Cancelled(reason),
            Never::Refused(refusal) => Outcome::Refused(refusal),
        };
        let end = outcome.end();
        self.outcome = Some(outcome);
        end
    }

    fn deliver(self: Box<Self>) {
        let Some(outcome) = self.outcome else {
            return;
        };
        match self.sink {
            Sink::Slot(slot) => *lock(&slot) = Some(outcome),
            Sink::Then(then) => then(outcome),
        }
    }
}

/// A panic's message: what `panic!` was given, when that was text.
fn panic_message(panic: &(dyn Any + Send)) -> String {
    if let Some(text) = panic.downcast_ref::<&'static str>() {
        (*text).to_owned()
    } else if let Some(text) = panic.downcast_ref::<String>() {
        text.clone()
    } else {
        "a panic that carried no message".to_owned()
    }
}
