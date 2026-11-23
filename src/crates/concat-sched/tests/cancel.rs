// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Cancelling: one job, everything an owner has, and what the job's
//! outcome says afterwards.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use common::{BOUND, BRIEF, Gate, budget, events, hold, next, outcome, over, small, state_of};
use concat_sched::{
    CancelReason, Config, End, Event, Job, JobContext, JobError, JobState, Lane, Outcome, Owner,
    Scheduler,
};

/// The inside of a real job: round and round, with a check each time and
/// nothing else that could see a cancel. Says on `started` that it is
/// going once it has been round once, and ends only by being cancelled.
fn spin<E>(ctx: &JobContext, started: &mpsc::Sender<()>) -> Result<(), JobError<E>> {
    let mut turns = 0u64;
    loop {
        ctx.checkpoint()?;
        turns += 1;
        if turns == 1 {
            started.send(()).expect("the test is listening");
        }
        std::hint::spin_loop();
    }
}

#[test]
fn a_job_cancelled_while_it_waits_never_runs_and_is_over_at_once() {
    let scheduler = small(1);
    let gate = Gate::new();
    let holder = hold(&scheduler, Lane::Background, &gate);

    let ran = Arc::new(AtomicBool::new(false));
    let waiting = {
        let ran = Arc::clone(&ran);
        scheduler.submit(Job::infallible(Lane::Background, move |_| {
            ran.store(true, Ordering::Release);
        }))
    };
    assert_eq!(state_of(&scheduler, &waiting), Some(JobState::Queued));

    assert!(waiting.cancel());
    // Nothing to wait for: the cancel itself ended it.
    assert!(waiting.is_finished());
    assert_eq!(waiting.end(), Some(End::Cancelled(CancelReason::Requested)));
    assert_eq!(state_of(&scheduler, &waiting), None);
    // A second cancel has nothing to do.
    assert!(!waiting.cancel());
    assert_eq!(
        outcome(waiting),
        Outcome::Cancelled(CancelReason::Requested)
    );

    gate.open();
    assert_eq!(outcome(holder), Outcome::Done(()));
    drop(scheduler);
    assert!(!ran.load(Ordering::Acquire));
}

#[test]
fn a_job_that_checks_in_its_loop_stops_when_it_is_cancelled() {
    let scheduler = small(1);
    let (started, starts) = mpsc::channel();
    let job = scheduler.submit(Job::new(Lane::Background, move |ctx| {
        spin::<()>(ctx, &started)
    }));
    // It is in its loop, past the first check.
    next(&starts);
    assert_eq!(state_of(&scheduler, &job), Some(JobState::Running));

    assert!(job.cancel());
    assert_eq!(outcome(job), Outcome::Cancelled(CancelReason::Requested));
    assert!(scheduler.snapshot().is_idle());
}

#[test]
fn a_job_asleep_on_its_token_is_woken_by_the_cancel() {
    let scheduler = small(1);
    let (started, starts) = mpsc::channel();
    let job = scheduler.submit(Job::new(Lane::Io, move |ctx| {
        started.send(()).expect("the test is listening");
        // A download between two tries. Without the cancel this would
        // outlast the test's bound and fail it.
        let cancelled = ctx.token().wait_timeout(BOUND * 4);
        ctx.checkpoint()?;
        Ok::<_, JobError<()>>(cancelled)
    }));
    next(&starts);
    job.cancel();
    assert_eq!(outcome(job), Outcome::Cancelled(CancelReason::Requested));
}

#[test]
fn a_cancel_wins_over_whatever_the_job_returns() {
    let scheduler = small(1);
    for fails in [false, true] {
        let (started, starts) = mpsc::channel();
        let job = scheduler.submit(Job::new(Lane::Io, move |ctx| {
            started.send(()).expect("the test is listening");
            ctx.token().wait_timeout(BOUND);
            // It does not look at its token again; it just finishes, one
            // way or the other.
            if fails {
                Err(JobError::Failed("the disk is full"))
            } else {
                Ok("a whole file")
            }
        }));
        next(&starts);
        job.cancel();
        // Never a result and never an error: it was cancelled.
        assert_eq!(outcome(job), Outcome::Cancelled(CancelReason::Requested));
    }
}

#[test]
fn work_that_answers_a_cancel_with_an_error_of_its_own_is_still_cancelled() {
    // The old shape: a flag polled somewhere deep, and "cancelled" coming
    // back as the text of an error. The text never reaches the caller.
    let scheduler = small(1);
    let (started, starts) = mpsc::channel();
    let job = scheduler.submit(Job::fallible(Lane::Background, move |ctx| {
        started.send(()).expect("the test is listening");
        let flag = ctx.token().as_flag();
        ctx.token().wait_timeout(BOUND);
        if flag.load(Ordering::Acquire) {
            return Err("cancelled - wait for it or cancel it first".to_owned());
        }
        Ok(())
    }));
    next(&starts);
    job.cancel();
    assert_eq!(outcome(job), Outcome::Cancelled(CancelReason::Requested));
}

#[test]
fn a_job_that_stops_itself_as_cancelled_ends_as_cancelled() {
    let scheduler = small(1);
    let job = scheduler.submit(Job::new(Lane::Io, |_| {
        Err::<(), JobError<String>>(JobError::Cancelled)
    }));
    assert_eq!(outcome(job), Outcome::Cancelled(CancelReason::Requested));
}

#[test]
fn a_cancel_after_the_job_is_over_changes_nothing() {
    let scheduler = small(1);
    let job = scheduler.submit(Job::infallible(Lane::Io, |_| 3));
    let ticket = job.ticket();
    assert_eq!(outcome(job), Outcome::Done(3));
    ticket.cancel();
    assert_eq!(ticket.end(), Some(End::Done));
    assert_eq!(ticket.state(), JobState::Ended);
}

#[test]
fn closing_an_owner_cancels_all_it_has_and_nothing_else() {
    let (sink, seen) = events();
    let scheduler = Scheduler::new(Config::new(budget(1, [1; Lane::COUNT])).events(sink));
    let project = Owner(1);
    let other_project = Owner(2);

    // One of the project's jobs is running, two are waiting behind it,
    // and one is on another lane altogether.
    let (started, starts) = mpsc::channel();
    let captions = scheduler.submit(
        Job::new(Lane::Background, move |ctx| spin::<String>(ctx, &started))
            .label("captions")
            .owner(project),
    );
    next(&starts);
    let gate = Gate::new();
    let download = hold(&scheduler, Lane::Io, &gate);
    let ran = Arc::new(AtomicBool::new(false));
    let mut waiting = Vec::new();
    for lane in [Lane::Background, Lane::Background, Lane::Io] {
        let ran = Arc::clone(&ran);
        waiting.push(scheduler.submit(
            Job::infallible(lane, move |_| ran.store(true, Ordering::Release)).owner(project),
        ));
    }
    let theirs =
        scheduler.submit(Job::infallible(Lane::Background, |_| "theirs").owner(other_project));
    let nobodys = scheduler.submit(Job::infallible(Lane::Background, |_| "nobody's"));

    let stopping = scheduler.cancel_owner(project);
    assert_eq!(stopping.len(), 4);
    // The three that were waiting are over already.
    for job in &waiting {
        assert!(job.is_finished());
    }
    // And the wait is for the one that was running.
    assert_eq!(stopping.wait(BOUND), Ok(()));
    assert!(captions.is_finished());
    assert!(
        scheduler
            .snapshot()
            .jobs()
            .all(|job| job.owner != Some(project))
    );

    assert_eq!(outcome(captions), Outcome::Cancelled(CancelReason::Owner));
    for job in waiting {
        assert_eq!(outcome(job), Outcome::Cancelled(CancelReason::Owner));
    }
    assert!(!ran.load(Ordering::Acquire));
    assert_eq!(outcome(theirs), Outcome::Done("theirs"));
    assert_eq!(outcome(nobodys), Outcome::Done("nobody's"));
    assert_eq!(state_of(&scheduler, &download), Some(JobState::Running));
    gate.open();
    assert_eq!(outcome(download), Outcome::Done(()));

    // Closing it again finds nothing, and waits for nothing.
    let again = scheduler.cancel_owner(project);
    assert!(again.is_empty());
    assert_eq!(again.wait(BRIEF), Ok(()));

    drop(scheduler);
    let cancelled = seen
        .try_iter()
        .filter(|event| {
            matches!(
                event,
                Event::Finished {
                    end: End::Cancelled(CancelReason::Owner),
                    ..
                }
            )
        })
        .count();
    assert_eq!(cancelled, 4);
}

#[test]
fn an_owners_waiting_jobs_do_not_start_as_its_running_ones_stop() {
    // The order that once went wrong: the running job is cancelled, it
    // stops and frees its thread, and the next job in line - the same
    // owner's, about to be cancelled itself - is started on it. Here the
    // cancel of the running job is made to take until that job is quite
    // over, so if the line were not emptied first, the next would run.
    let scheduler = small(1);
    let project = Owner(3);
    let (started, starts) = mpsc::channel();
    let running = scheduler
        .submit(Job::new(Lane::Background, move |ctx| spin::<()>(ctx, &started)).owner(project));
    next(&starts);
    let over_first = running.ticket();
    running.token().on_cancel(move |_| over(&over_first));

    let ran = Arc::new(AtomicBool::new(false));
    let waiting: Vec<_> = (0..3)
        .map(|_| {
            let ran = Arc::clone(&ran);
            scheduler.submit(
                Job::infallible(Lane::Background, move |_| {
                    ran.store(true, Ordering::Release)
                })
                .owner(project),
            )
        })
        .collect();

    let stopping = scheduler.cancel_owner(project);
    assert_eq!(stopping.len(), 4);
    assert_eq!(stopping.wait(BOUND), Ok(()));
    assert_eq!(outcome(running), Outcome::Cancelled(CancelReason::Owner));
    for job in waiting {
        assert_eq!(outcome(job), Outcome::Cancelled(CancelReason::Owner));
    }
    drop(scheduler);
    assert!(!ran.load(Ordering::Acquire));
}

#[test]
fn waiting_on_a_closed_owner_names_the_job_that_ignores_its_token() {
    let scheduler = small(1);
    let gate = Gate::new();
    let project = Owner(9);
    let (started, starts) = mpsc::channel();
    let stubborn = {
        let gate = gate.clone();
        scheduler.submit(
            Job::infallible(Lane::Background, move |_| {
                started.send(()).expect("the test is listening");
                gate.pass();
            })
            .label("stubborn")
            .owner(project),
        )
    };
    next(&starts);

    let stopping = scheduler.cancel_owner(project);
    // It cannot end before the gate opens, so this wait runs out however
    // long it is, and says who it was waiting for.
    let late = stopping
        .wait(BRIEF)
        .expect_err("the job is still at the gate");
    assert_eq!(late.len(), 1);
    assert_eq!(late[0].id, stubborn.id());
    assert_eq!(late[0].label, "stubborn");
    assert_eq!(late[0].state, JobState::Stopping);

    gate.open();
    assert_eq!(stopping.wait(BOUND), Ok(()));
    // It ran to its end, and it is cancelled all the same.
    assert_eq!(outcome(stubborn), Outcome::Cancelled(CancelReason::Owner));
}

#[test]
fn a_job_standing_aside_is_woken_by_its_cancel() {
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
    let gate = Gate::new();
    let (export, _) = common::hold_wide(&scheduler, Lane::Foreground, &gate, usize::MAX);
    go.send(()).expect("the job is waiting");
    loop {
        if matches!(next(&seen), Event::Paused(job) if job.id == analysis.id()) {
            break;
        }
    }
    assert_eq!(state_of(&scheduler, &analysis), Some(JobState::Paused));

    // The export is still running; the cancel is what lets it out.
    analysis.cancel();
    assert_eq!(
        outcome(analysis),
        Outcome::Cancelled(CancelReason::Requested)
    );
    assert_eq!(state_of(&scheduler, &export), Some(JobState::Running));
    assert_eq!(scheduler.snapshot().cores, 2);
    gate.open();
    assert_eq!(outcome(export), Outcome::Done(()));
}

#[test]
fn the_callback_is_told_once_how_the_job_ended_run_or_not() {
    let scheduler = small(1);
    let (told, tellings) = mpsc::channel();

    let done = {
        let told = told.clone();
        scheduler.submit_then(Job::infallible(Lane::Io, |_| 5), move |outcome| {
            told.send(outcome).expect("the test is listening");
        })
    };
    assert_eq!(next(&tellings), Outcome::Done(5));
    over(&done);

    let gate = Gate::new();
    let holder = hold(&scheduler, Lane::Io, &gate);
    let waiting = scheduler.submit_then(Job::infallible(Lane::Io, |_| 6), move |outcome| {
        told.send(outcome).expect("the test is listening");
    });
    waiting.cancel();
    // Told on this thread, inside the cancel.
    assert_eq!(
        tellings.try_recv(),
        Ok(Outcome::Cancelled(CancelReason::Requested))
    );
    gate.open();
    assert_eq!(outcome(holder), Outcome::Done(()));
    drop(scheduler);
    assert!(tellings.try_recv().is_err());
}

#[test]
fn a_hook_on_a_jobs_token_runs_when_the_job_is_cancelled() {
    let scheduler = small(1);
    let (started, starts) = mpsc::channel();
    let (aborted, aborts) = mpsc::channel();
    let job = scheduler.submit(Job::new(Lane::Io, move |ctx| {
        // A request that only its own abort can stop.
        ctx.token().on_cancel(move |reason| {
            aborted.send(reason).expect("the job is listening");
        });
        started.send(()).expect("the test is listening");
        let reason = aborts.recv_timeout(BOUND).expect("the abort comes");
        ctx.checkpoint()?;
        Ok::<_, JobError<()>>(reason)
    }));
    next(&starts);
    job.cancel();
    assert_eq!(outcome(job), Outcome::Cancelled(CancelReason::Requested));
}
