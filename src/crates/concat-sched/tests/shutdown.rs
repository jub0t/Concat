// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Shutting down: everything is cancelled, every thread is joined within
//! a bound, and a job that would not stop is named.

mod common;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use common::{BOUND, BRIEF, Gate, budget, events, hold_wide, next, outcome, over, small};
use concat_sched::{
    CancelReason, Config, End, Event, Job, JobError, JobState, Lane, Outcome, Refusal, Scheduler,
};

#[test]
fn a_shutdown_cancels_what_waits_and_what_runs_and_joins_every_thread() {
    let left = Arc::new(AtomicUsize::new(0));
    let names = Arc::new(Mutex::new(Vec::new()));
    let scheduler = {
        let names = Arc::clone(&names);
        Scheduler::new(
            Config::new(budget(1, [1, 1, 1, 2, 1, 2])).on_thread_start(move |lane| {
                names
                    .lock()
                    .expect("the names' lock")
                    .push((lane, std::thread::current().name().map(str::to_owned)));
            }),
        )
    };
    assert_eq!(scheduler.budget().threads(), 8);

    // A job that ends when asked, and counts itself out as it goes.
    let (started, starts) = mpsc::channel();
    let running = {
        let left = Arc::clone(&left);
        scheduler.submit(Job::new(Lane::Background, move |ctx| {
            started.send(()).expect("the test is listening");
            ctx.token().wait_timeout(BOUND);
            left.fetch_add(1, Ordering::AcqRel);
            ctx.checkpoint()?;
            Ok::<_, JobError<()>>(())
        }))
    };
    next(&starts);
    let ran = Arc::new(AtomicBool::new(false));
    let waiting = {
        let ran = Arc::clone(&ran);
        scheduler.submit(Job::infallible(Lane::Background, move |_| {
            ran.store(true, Ordering::Release);
        }))
    };

    let report = scheduler.shutdown(BOUND);
    assert!(report.is_clean(), "left behind: {:?}", report.ignored);
    // Joined, not merely asked: the running job has been through its end.
    assert_eq!(left.load(Ordering::Acquire), 1);
    assert_eq!(outcome(running), Outcome::Cancelled(CancelReason::Shutdown));
    assert_eq!(outcome(waiting), Outcome::Cancelled(CancelReason::Shutdown));
    assert!(!ran.load(Ordering::Acquire));
    assert!(scheduler.snapshot().is_idle());

    // Every thread started, on its own lane, under its own name.
    let mut names = names.lock().expect("the names' lock").clone();
    names.sort();
    let names: Vec<(Lane, &str)> = names
        .iter()
        .map(|(lane, name)| (*lane, name.as_deref().unwrap_or("")))
        .collect();
    assert_eq!(
        names,
        [
            (Lane::Frame, "concat-frame-0"),
            (Lane::Save, "concat-save-0"),
            (Lane::Foreground, "concat-foreground-0"),
            (Lane::Prefetch, "concat-prefetch-0"),
            (Lane::Prefetch, "concat-prefetch-1"),
            (Lane::Background, "concat-background-0"),
            (Lane::Io, "concat-io-0"),
            (Lane::Io, "concat-io-1"),
        ]
    );
}

#[test]
fn nothing_is_taken_after_a_shutdown() {
    let scheduler = small(1);
    assert!(scheduler.shutdown(BOUND).is_clean());
    let ran = Arc::new(AtomicBool::new(false));
    let late = {
        let ran = Arc::clone(&ran);
        scheduler.submit(Job::infallible(Lane::Save, move |_| {
            ran.store(true, Ordering::Release);
        }))
    };
    assert!(late.is_finished());
    assert_eq!(outcome(late), Outcome::Refused(Refusal::ShutDown));
    assert!(!ran.load(Ordering::Acquire));
    // A second shutdown has nothing to do and nothing to report.
    assert!(scheduler.shutdown(BRIEF).is_clean());
}

#[test]
fn a_shutdown_names_the_job_that_ignored_its_token_and_does_not_wait_for_it() {
    let scheduler = small(1);
    let gate = Gate::new();
    let (stubborn, _) = hold_wide(&scheduler, Lane::Background, &gate, 1);
    let obedient = {
        let (started, starts) = mpsc::channel();
        let job = scheduler.submit(Job::new(Lane::Io, move |ctx| {
            started.send(()).expect("the test is listening");
            ctx.token().wait_timeout(BOUND);
            ctx.checkpoint()?;
            Ok::<_, JobError<()>>(())
        }));
        next(&starts);
        job
    };

    // The stubborn job is at a gate nobody has opened, so it cannot have
    // stopped however long the bound, and the shutdown comes back
    // without it, saying so.
    let report = scheduler.shutdown(BRIEF);
    let named = report
        .ignored
        .iter()
        .find(|job| job.id == stubborn.id())
        .expect("the job at the gate is in the report");
    assert_eq!(named.label, "held");
    assert_eq!(named.state, JobState::Stopping);

    // The other job stops when asked. Once it has, a second shutdown
    // reports the stubborn one and nothing else.
    assert_eq!(
        outcome(obedient),
        Outcome::Cancelled(CancelReason::Shutdown)
    );
    let again = scheduler.shutdown(BRIEF);
    assert_eq!(again.ignored.len(), 1);
    assert_eq!(again.ignored[0].id, stubborn.id());

    // Its thread was left to it, and when it does end it ends in order.
    gate.open();
    assert_eq!(
        outcome(stubborn),
        Outcome::Cancelled(CancelReason::Shutdown)
    );
    assert!(scheduler.shutdown(BOUND).is_clean());
}

#[test]
fn dropping_the_scheduler_shuts_it_down() {
    let (sink, seen) = events();
    let scheduler = Scheduler::new(Config::new(budget(1, [1; Lane::COUNT])).events(sink));
    let (started, starts) = mpsc::channel();
    let running = scheduler.submit(Job::new(Lane::Foreground, move |ctx| {
        started.send(()).expect("the test is listening");
        ctx.token().wait();
        ctx.checkpoint()?;
        Ok::<_, JobError<()>>(())
    }));
    next(&starts);
    let waiting = scheduler.submit(Job::infallible(Lane::Foreground, |_| ()));
    let (running_id, waiting_id) = (running.id(), waiting.id());

    drop(scheduler);
    // The handles outlive it, and both jobs are over.
    assert!(running.is_finished());
    assert!(waiting.is_finished());
    assert_eq!(outcome(running), Outcome::Cancelled(CancelReason::Shutdown));
    assert_eq!(outcome(waiting), Outcome::Cancelled(CancelReason::Shutdown));

    let ends: Vec<_> = seen
        .try_iter()
        .filter_map(|event| match event {
            Event::Finished { job, end, ran } => Some((job.id, end, ran.is_some())),
            _ => None,
        })
        .collect();
    let shutdown = End::Cancelled(CancelReason::Shutdown);
    assert_eq!(ends.len(), 2);
    assert!(ends.contains(&(waiting_id, shutdown.clone(), false)));
    assert!(ends.contains(&(running_id, shutdown, true)));
}

#[test]
fn a_job_standing_aside_is_let_out_by_a_shutdown() {
    let (sink, seen) = events();
    let scheduler = Scheduler::new(Config::new(budget(2, [1; Lane::COUNT])).events(sink));
    let (started, starts) = mpsc::channel();
    let (go, goes) = mpsc::channel::<()>();
    let analysis = scheduler.submit(
        Job::new(Lane::Background, move |ctx| {
            started.send(()).expect("the test is listening");
            goes.recv_timeout(BOUND).expect("the test says when");
            ctx.checkpoint()?;
            Ok::<_, JobError<()>>(())
        })
        .wide(),
    );
    next(&starts);
    // An export that ends when it is told to, on the same cores.
    let (begun, begins) = mpsc::channel();
    let export = scheduler.submit(
        Job::new(Lane::Foreground, move |ctx| {
            begun.send(()).expect("the test is listening");
            ctx.token().wait();
            ctx.checkpoint()?;
            Ok::<_, JobError<()>>(())
        })
        .wide(),
    );
    next(&begins);
    go.send(()).expect("the job is waiting");
    loop {
        if matches!(next(&seen), Event::Paused(job) if job.id == analysis.id()) {
            break;
        }
    }

    assert!(scheduler.shutdown(BOUND).is_clean());
    assert_eq!(
        outcome(analysis),
        Outcome::Cancelled(CancelReason::Shutdown)
    );
    assert_eq!(outcome(export), Outcome::Cancelled(CancelReason::Shutdown));
}

#[test]
fn a_shutdown_from_inside_a_job_does_not_wait_for_that_job() {
    let scheduler = Arc::new(small(1));
    let inside = Arc::clone(&scheduler);
    let (reported, reports) = mpsc::channel();
    let job = scheduler.submit_then(
        Job::infallible(Lane::Io, move |ctx| {
            let report = inside.shutdown(BOUND);
            reported
                .send((ctx.id(), report))
                .expect("the test is listening");
        }),
        |_| {},
    );
    let (id, report) = next(&reports);
    // Every other thread is joined; the one left is the one that asked.
    assert_eq!(report.ignored.len(), 1);
    assert_eq!(report.ignored[0].id, id);
    over(&job);
    assert!(scheduler.shutdown(BOUND).is_clean());
}
