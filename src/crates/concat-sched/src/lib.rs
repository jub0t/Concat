// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The one scheduler: every piece of work that is not the window's own
//! thread or the mixer's goes through here.
//!
//! The editor had a pool of threads and, beside it, a slot per kind of
//! long job, each with its own idea of what a cancel is and how much of
//! the machine it may take. Four faults came of that, and this crate is
//! built so that none of them can be written again:
//!
//! - **The save waited behind an export.** Everything shared one queue,
//!   so three long jobs held the monitor's frames and the autosave for
//!   minutes, and a crash then lost every edit since the last save. Here
//!   work is in [`Lane`]s, the frame and the save each have threads
//!   nothing else is run on, and a lane never waits for a lane below it.
//! - **A cancel was a word in an error.** It was recognised by looking
//!   for "cancel" in the text, and "wait for it or cancel it first"
//!   matched. Here a job has a [`CancelToken`], and being cancelled is an
//!   ending of its own, [`Outcome::Cancelled`], with the reason as a
//!   type. Closing a project cancels all that the project owns
//!   ([`Scheduler::cancel_owner`]) and can wait until it has stopped.
//! - **A failed analysis was started again by every edit.** Here a job
//!   says what it was run on ([`Job::input`]), a failure is remembered
//!   under that, and the next job on the same input is answered at once
//!   with [`Outcome::FailedBefore`].
//! - **Nothing owned the machine.** Each job picked its own number of
//!   threads; two cores were asked to run sixty and thirty-two cores
//!   were left mostly idle. Here one [`Budget`] is worked out from the
//!   [`Machine`], and a job is told how many cores are its own
//!   ([`JobContext::threads`]).
//!
//! # The lanes
//!
//! Most urgent first: the [frame](Lane::Frame) on screen; the
//! [save](Lane::Save); [foreground](Lane::Foreground) jobs, which the
//! person is waiting on; [prefetch](Lane::Prefetch);
//! [background](Lane::Background) analysis and models; and
//! [file and network](Lane::Io) work. Sound is not among them: it is
//! mixed on a real-time thread that the output device calls, and is
//! never scheduled.
//!
//! Each lane has threads of its own, so there is always a thread for a
//! frame and one for a save. The frame, the save and the file and network
//! lane are limited by those threads alone. The three in the middle also
//! share one pool of cores, and the rule for it is short:
//!
//! - **A lane never waits for a lane below it.** What the lanes below
//!   hold is not counted when a job above asks for cores. The job starts
//!   with what it is due, at once.
//! - **A lane below may use whatever the lanes above are not using**,
//!   apart from the little kept for the lanes below itself. On an idle
//!   machine an analysis has the whole pool.
//! - **It gives that back at its next checkpoint.** When a lane above
//!   starts a job on cores a lane below is using, the newest jobs below
//!   are asked to stand aside; at their next
//!   [`JobContext::checkpoint`] they give their cores up and block until
//!   the cores are free again. A job is asked when at least half of what
//!   it holds is wanted above; for less than that it runs on and the
//!   machine is that much over-full, since standing aside is all of a
//!   job or none of it. A job that does not call the checkpoint cannot
//!   be held at all, and keeps its cores until it ends - so such a job
//!   should ask for few.
//! - **Each lane keeps a small reserve** ([`LaneBudget::reserve`]) that
//!   the lanes above leave alone, on machines with cores to spare, so the
//!   picture still plays and the cards still fill in under an export.
//!
//! # A job
//!
//! A [`Job`] is its work and what is said about it: its [`Owner`], a
//! [`JobKey`] with what to do about a second job of the same key
//! ([`Coalesce`]), the [`InputKey`] its failure is remembered under, the
//! [`Resource`]s it must hold, and how many cores it can use. The work is
//! handed a [`JobContext`]: the token to check, the progress to report,
//! the cores it was granted. What comes back is a [`JobHandle`], or with
//! [`Scheduler::submit_then`] a [`Ticket`] and a call when it is over;
//! either way the ending is an [`Outcome`].
//!
//! ```
//! use concat_sched::{Budget, Config, Job, JobError, Lane, Machine, Outcome, Scheduler};
//!
//! let machine = Machine::new(8, 16 << 30);
//! let scheduler = Scheduler::new(Config::new(Budget::for_machine(machine)));
//!
//! let job = scheduler.submit(
//!     Job::new(Lane::Background, |ctx| {
//!         let mut sum = 0u64;
//!         for step in 0..1000u64 {
//!             ctx.checkpoint()?;
//!             sum += step;
//!             ctx.progress(step as f32 / 1000.0);
//!         }
//!         Ok::<_, JobError<String>>(sum)
//!     })
//!     .label("sum"),
//! );
//! assert_eq!(job.wait(), Outcome::Done(499_500));
//! ```
//!
//! # What is bounded
//!
//! Nothing here grows without a limit that someone chose. A lane's queue
//! holds so many jobs and then refuses ([`Config::queue_limit`]); a job
//! running is one of its lane's threads; the failures remembered are so
//! many and then the oldest goes ([`Config::failure_memory`]); a job's
//! cancel hooks go when the job ends. Events are not kept at all: they
//! are handed to the host's function as they happen.
//!
//! Nothing here polls, either. A thread with nothing to do waits on a
//! condition and is woken by the change it is waiting for; there is no
//! sleep in the crate and no loop that spins.
//!
//! # Panics
//!
//! A job that panics ends as [`Outcome::Panicked`]; its thread goes on to
//! the next job, what it held is given back, and the scheduler is
//! untouched. So does a callback or an events function that panics.
//!
//! That is true of a build that unwinds, and only of one: the tests, the
//! command line, the server. **The window is built with `panic =
//! "abort"`**, and there a panic anywhere ends the process before
//! anything can catch it. Nothing in this crate changes that; work that
//! must not be able to take the window down belongs in the helper
//! process, not behind a `catch_unwind`.

#![forbid(unsafe_code)]

mod budget;
mod cancel;
mod job;
mod memory;
mod scheduler;
mod watch;

pub use budget::{Budget, Lane, LaneBudget, Machine};
pub use cancel::{CancelReason, CancelToken, Cancelled};
pub use job::{
    Coalesce, End, InputKey, Job, JobContext, JobError, JobHandle, JobId, JobInfo, JobKey,
    JobState, OrFail, Outcome, Owner, Progress, Refusal, Resource, Ticket,
};
pub use scheduler::{Config, Scheduler, ShutdownReport, Stopping};
pub use watch::{Event, LaneLoad, LaneSnapshot, ResourceUse, Snapshot};

use std::sync::{Mutex, MutexGuard};

/// Takes a lock, poisoned or not.
///
/// A lock is poisoned when a thread panicked while holding it. Nothing of
/// a caller's runs under any lock in this crate - not a job, a callback,
/// a hook or an events function - so that would be a fault in the crate
/// itself, and what the lock guards is still the best account there is of
/// what is running. Carrying on with it keeps the other jobs going.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
