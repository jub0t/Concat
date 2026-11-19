// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Two jobs with one key: the newest wins, the first wins, or both run.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use common::{BOUND, Gate, budget, hold, log, named, next, outcome, small, state_of, written};
use concat_sched::{
    CancelReason, Coalesce, Config, Job, JobError, JobKey, JobState, Lane, Outcome, Refusal,
    Scheduler,
};

#[test]
fn a_newer_job_replaces_one_still_waiting_which_never_runs() {
    let scheduler = small(1);
    let gate = Gate::new();
    let holder = hold(&scheduler, Lane::Save, &gate);

    let ran = Arc::new(AtomicBool::new(false));
    let older = {
        let ran = Arc::clone(&ran);
        scheduler.submit(
            Job::infallible(Lane::Save, move |_| {
                ran.store(true, Ordering::Release);
                "older"
            })
            .key("save a.concat", Coalesce::Replace),
        )
    };
    let newer = scheduler
        .submit(Job::infallible(Lane::Save, |_| "newer").key("save a.concat", Coalesce::Replace));

    // The older one is over before the submit that replaced it returned.
    assert!(older.is_finished());
    assert_eq!(outcome(older), Outcome::Cancelled(CancelReason::Replaced));
    assert_eq!(scheduler.snapshot().lane(Lane::Save).queued.len(), 1);

    gate.open();
    assert_eq!(outcome(holder), Outcome::Done(()));
    assert_eq!(outcome(newer), Outcome::Done("newer"));
    assert!(!ran.load(Ordering::Acquire));
}

#[test]
fn a_newer_job_takes_the_place_in_line_of_the_one_it_replaces() {
    let scheduler = small(1);
    let gate = Gate::new();
    let holder = hold(&scheduler, Lane::Save, &gate);

    let log = log();
    let older =
        scheduler.submit(named(Lane::Save, "a, older", &log).key("save a", Coalesce::Replace));
    let other = scheduler.submit(named(Lane::Save, "b", &log).key("save b", Coalesce::Replace));
    let newer =
        scheduler.submit(named(Lane::Save, "a, newer", &log).key("save a", Coalesce::Replace));

    gate.open();
    assert_eq!(outcome(holder), Outcome::Done(()));
    assert_eq!(outcome(older), Outcome::Cancelled(CancelReason::Replaced));
    assert_eq!(outcome(other), Outcome::Done(()));
    assert_eq!(outcome(newer), Outcome::Done(()));
    // Not sent to the back for being replaced: a save that is asked for
    // again and again must not be the one that never gets written.
    assert_eq!(written(&log), ["a, newer", "b"]);
}

#[test]
fn a_newer_job_cancels_the_one_running_and_waits_for_it_to_stop() {
    // Two threads, so only the key keeps the two apart.
    let scheduler = Scheduler::new(Config::new(budget(2, [2; Lane::COUNT])));
    let (started, starts) = mpsc::channel();
    let (stopping, stops) = mpsc::channel();
    let linger = Gate::new();
    let log = log();

    let older = {
        let linger = linger.clone();
        let log = Arc::clone(&log);
        scheduler.submit(
            Job::new(Lane::Frame, move |ctx| {
                started.send(()).expect("the test is listening");
                ctx.token().wait_timeout(BOUND);
                stopping
                    .send(ctx.token().reason())
                    .expect("the test is listening");
                // Still tidying up after itself.
                linger.pass();
                log.lock().expect("the log's lock").push("older, leaving");
                ctx.checkpoint()?;
                Ok::<_, JobError<()>>(())
            })
            .key("frame", Coalesce::Replace),
        )
    };
    next(&starts);

    let newer = scheduler.submit(named(Lane::Frame, "newer", &log).key("frame", Coalesce::Replace));
    assert_eq!(next(&stops), Some(CancelReason::Replaced));
    // A thread is idle, and the newer job is not on it: the older one has
    // not gone yet.
    assert_eq!(state_of(&scheduler, &older), Some(JobState::Stopping));
    assert_eq!(state_of(&scheduler, &newer), Some(JobState::Queued));
    // Another frame-lane job is not held up by the two of them.
    let other = scheduler.submit(Job::infallible(Lane::Frame, |_| ()));
    assert_eq!(outcome(other), Outcome::Done(()));

    linger.open();
    assert_eq!(outcome(older), Outcome::Cancelled(CancelReason::Replaced));
    assert_eq!(outcome(newer), Outcome::Done(()));
    assert_eq!(written(&log), ["older, leaving", "newer"]);
}

#[test]
fn a_job_that_keeps_the_first_is_refused_while_the_first_is_waiting_or_running() {
    let scheduler = small(1);
    let gate = Gate::new();
    let export = {
        let gate = gate.clone();
        let (started, starts) = mpsc::channel();
        let export = scheduler.submit(
            Job::infallible(Lane::Foreground, move |_| {
                started.send(()).expect("the test is listening");
                gate.pass();
                "exported"
            })
            .key("export", Coalesce::KeepFirst),
        );
        next(&starts);
        export
    };

    // A second press while it runs...
    let again = scheduler
        .submit(Job::infallible(Lane::Foreground, |_| "again").key("export", Coalesce::KeepFirst));
    assert!(again.is_finished());
    assert_eq!(
        outcome(again),
        Outcome::Refused(Refusal::Duplicate(export.id()))
    );

    // ...and while one waits behind it.
    let merge = scheduler
        .submit(Job::infallible(Lane::Foreground, |_| "merged").key("merge", Coalesce::KeepFirst));
    assert_eq!(state_of(&scheduler, &merge), Some(JobState::Queued));
    let merge_again = scheduler.submit(
        Job::infallible(Lane::Foreground, |_| "merged again").key("merge", Coalesce::KeepFirst),
    );
    assert_eq!(
        outcome(merge_again),
        Outcome::Refused(Refusal::Duplicate(merge.id()))
    );

    gate.open();
    assert_eq!(outcome(export), Outcome::Done("exported"));
    assert_eq!(outcome(merge), Outcome::Done("merged"));

    // Once the first is over the key is free again.
    let later = scheduler
        .submit(Job::infallible(Lane::Foreground, |_| "later").key("export", Coalesce::KeepFirst));
    assert_eq!(outcome(later), Outcome::Done("later"));
}

#[test]
fn jobs_that_run_both_run_side_by_side_under_one_key() {
    let scheduler = Scheduler::new(Config::new(budget(2, [2; Lane::COUNT])));
    let gate = Gate::new();
    let (started, starts) = mpsc::channel();
    let jobs: Vec<_> = (0..2)
        .map(|index| {
            let gate = gate.clone();
            let started = started.clone();
            scheduler.submit(
                Job::infallible(Lane::Io, move |_| {
                    started.send(index).expect("the test is listening");
                    gate.pass();
                    index
                })
                .key("download", Coalesce::RunBoth),
            )
        })
        .collect();

    // Both have started, and neither can end until the gate opens.
    let mut running = [next(&starts), next(&starts)];
    running.sort_unstable();
    assert_eq!(running, [0, 1]);
    assert_eq!(scheduler.snapshot().lane(Lane::Io).running.len(), 2);

    gate.open();
    let ended: Vec<_> = jobs.into_iter().map(outcome).collect();
    assert_eq!(ended, [Outcome::Done(0), Outcome::Done(1)]);
}

#[test]
fn a_key_cancels_every_job_that_has_it_and_no_other() {
    let scheduler = Scheduler::new(Config::new(budget(2, [2; Lane::COUNT])));
    let gate = Gate::new();
    let (started, starts) = mpsc::channel();
    let running = scheduler.submit(
        Job::new(Lane::Io, move |ctx| {
            started.send(()).expect("the test is listening");
            ctx.token().wait_timeout(BOUND);
            ctx.checkpoint()?;
            Ok::<_, JobError<()>>(())
        })
        .key("download voice", Coalesce::RunBoth),
    );
    next(&starts);
    let other = hold(&scheduler, Lane::Io, &gate);
    let waiting = scheduler
        .submit(Job::infallible(Lane::Io, |_| ()).key("download voice", Coalesce::RunBoth));
    assert_eq!(state_of(&scheduler, &waiting), Some(JobState::Queued));

    let stopping = scheduler.cancel_key(&JobKey::new("download voice"));
    assert_eq!(stopping.len(), 2);
    assert_eq!(stopping.wait(BOUND), Ok(()));
    assert_eq!(
        outcome(running),
        Outcome::Cancelled(CancelReason::Requested)
    );
    assert_eq!(
        outcome(waiting),
        Outcome::Cancelled(CancelReason::Requested)
    );

    assert_eq!(state_of(&scheduler, &other), Some(JobState::Running));
    gate.open();
    assert_eq!(outcome(other), Outcome::Done(()));
    assert!(
        scheduler
            .cancel_key(&JobKey::new("download voice"))
            .is_empty()
    );
}
