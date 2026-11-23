// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! What the scheduler's tests share.
//!
//! No test here sleeps to let something happen. A job is put where the
//! test wants it by a [`Gate`] it waits at or a channel it reports on,
//! and the scheduler decides everything under its lock before the call
//! that caused the decision returns, so a snapshot taken straight after
//! a submit is the answer and not a guess. Every wait has [`BOUND`] on
//! it, and reaching the bound fails the test.

#![allow(dead_code)]

use std::convert::Infallible;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use concat_sched::{
    Budget, Config, Event, Job, JobHandle, JobState, Lane, LaneBudget, Outcome, Scheduler, Ticket,
};

/// Far longer than anything here takes. A wait that lasts this long has
/// failed, and says so.
pub const BOUND: Duration = Duration::from_secs(30);

/// Short, for the waits that are meant to run out: a job held at a gate
/// cannot end before the gate opens, however long is waited.
pub const BRIEF: Duration = Duration::from_millis(20);

/// Where a job waits until the test lets it through.
#[derive(Clone, Default)]
pub struct Gate {
    open: Arc<(Mutex<bool>, Condvar)>,
}

impl Gate {
    pub fn new() -> Gate {
        Gate::default()
    }

    /// Lets through everything waiting, and everything that comes after.
    pub fn open(&self) {
        let (open, changed) = &*self.open;
        *open.lock().expect("the gate's lock") = true;
        changed.notify_all();
    }

    /// Blocks until the gate is open.
    pub fn pass(&self) {
        let (open, changed) = &*self.open;
        let deadline = Instant::now() + BOUND;
        let mut is_open = open.lock().expect("the gate's lock");
        while !*is_open {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "the gate was never opened");
            is_open = changed
                .wait_timeout(is_open, left)
                .expect("the gate's lock")
                .0;
        }
    }
}

/// A budget by hand: `pool` cores, and each lane's threads in
/// [`Lane::ALL`] order. Nothing is kept for any lane, and a job may be as
/// wide as the pool.
pub fn budget(pool: usize, workers: [usize; Lane::COUNT]) -> Budget {
    Budget {
        cores: pool + 1,
        pool,
        lanes: workers.map(|workers| LaneBudget {
            workers,
            reserve: 0,
            width: pool,
        }),
    }
    .checked()
}

/// A scheduler with `pool` cores and one thread on every lane.
pub fn small(pool: usize) -> Scheduler {
    Scheduler::new(Config::new(budget(pool, [1; Lane::COUNT])))
}

/// What a channel has next, or a failed test if nothing comes.
pub fn next<T>(from: &Receiver<T>) -> T {
    from.recv_timeout(BOUND)
        .expect("something was to arrive on the channel, and nothing did")
}

/// How a job ended, or a failed test if it does not end.
pub fn outcome<T, E>(handle: JobHandle<T, E>) -> Outcome<T, E> {
    match handle.wait_timeout(BOUND) {
        Ok(outcome) => outcome,
        Err(handle) => panic!("{handle:?} did not end"),
    }
}

/// Blocks until `ticket`'s job is over, or fails the test.
pub fn over(ticket: &Ticket) {
    assert!(ticket.wait(BOUND).is_some(), "{ticket:?} did not end");
}

/// Starts a job on `lane` that asks for `threads` cores and then waits at
/// `gate`. Returns once it is running, with its handle and the cores it
/// was granted.
pub fn hold_wide(
    scheduler: &Scheduler,
    lane: Lane,
    gate: &Gate,
    threads: usize,
) -> (JobHandle<(), Infallible>, usize) {
    let (started, starts) = mpsc::channel();
    let gate = gate.clone();
    let handle = scheduler.submit(
        Job::infallible(lane, move |ctx| {
            started.send(ctx.threads()).expect("the test is listening");
            gate.pass();
        })
        .label("held")
        .threads(threads),
    );
    let granted = next(&starts);
    (handle, granted)
}

/// [`hold_wide`] for a job that asks for one core.
pub fn hold(scheduler: &Scheduler, lane: Lane, gate: &Gate) -> JobHandle<(), Infallible> {
    hold_wide(scheduler, lane, gate, 1).0
}

/// A job that only writes its name down when it runs.
pub fn named(lane: Lane, name: &'static str, log: &Arc<Mutex<Vec<&'static str>>>) -> Job<()> {
    let log = Arc::clone(log);
    Job::infallible(lane, move |_| {
        log.lock().expect("the log's lock").push(name);
    })
    .label(name)
}

/// A list for [`named`] jobs to write in.
pub fn log() -> Arc<Mutex<Vec<&'static str>>> {
    Arc::new(Mutex::new(Vec::new()))
}

/// What the [`named`] jobs wrote, in order.
pub fn written(log: &Arc<Mutex<Vec<&'static str>>>) -> Vec<&'static str> {
    log.lock().expect("the log's lock").clone()
}

/// An events function and the channel it writes to.
pub fn events() -> (impl Fn(&Event) + Send + Sync + 'static, Receiver<Event>) {
    let (send, receive): (Sender<Event>, Receiver<Event>) = mpsc::channel();
    let send = Mutex::new(send);
    (
        move |event: &Event| {
            // The test may have stopped listening; that is its business.
            let _ = send.lock().expect("the sender's lock").send(event.clone());
        },
        receive,
    )
}

/// Where `ticket`'s job is, as a snapshot of the scheduler has it.
pub fn state_of(scheduler: &Scheduler, ticket: &Ticket) -> Option<JobState> {
    scheduler.snapshot().job(ticket.id()).map(|job| job.state)
}
