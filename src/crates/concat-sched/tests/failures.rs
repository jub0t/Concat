// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Failures: an input that failed is not run again, and a job that
//! panics takes nothing down with it.
//!
//! These are tests of a build that unwinds, as every test build is. The
//! window is built with `panic = "abort"`, where a panic ends the process
//! and none of the second half applies; the crate's own documentation
//! says so.

mod common;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, UNIX_EPOCH};

use common::{BOUND, budget, events, next, outcome, over, small};
use concat_sched::{
    CancelReason, Config, End, Event, InputKey, Job, JobError, JobHandle, Lane, Outcome, Scheduler,
};

/// The error of the jobs here: why a cutout could not be made.
#[derive(Clone, PartialEq, Eq, Debug)]
enum CutoutError {
    Offline,
}

/// A cutout of `input` that counts its runs in `runs` and fails while
/// `online` is false.
fn cutout(
    scheduler: &Scheduler,
    input: &InputKey,
    runs: &Arc<AtomicUsize>,
    online: bool,
) -> JobHandle<&'static str, CutoutError> {
    let runs = Arc::clone(runs);
    scheduler.submit(
        Job::fallible(Lane::Background, move |_| {
            runs.fetch_add(1, Ordering::AcqRel);
            if online {
                Ok("a matte")
            } else {
                Err(CutoutError::Offline)
            }
        })
        .label("cutout")
        .input(input.clone()),
    )
}

#[test]
fn an_input_that_failed_is_not_run_again_and_is_answered_at_once() {
    let scheduler = small(1);
    let runs = Arc::new(AtomicUsize::new(0));
    let clip = InputKey::new("cutout 1|clip.mp4|1024|7");

    assert!(!scheduler.failed_before(&clip));
    let first = cutout(&scheduler, &clip, &runs, false);
    assert_eq!(outcome(first), Outcome::Failed(CutoutError::Offline));
    assert!(scheduler.failed_before(&clip));
    assert_eq!(scheduler.snapshot().failures, 1);

    // Every edit after that asks again. None of them runs anything, each
    // is over before its submit returns, and each says it is not news.
    for _ in 0..5 {
        let again = cutout(&scheduler, &clip, &runs, true);
        assert!(again.is_finished());
        assert_eq!(again.end(), Some(End::FailedBefore));
        assert_eq!(outcome(again), Outcome::FailedBefore(CutoutError::Offline));
    }
    assert_eq!(runs.load(Ordering::Acquire), 1);
}

#[test]
fn a_changed_input_is_another_input() {
    let scheduler = small(1);
    let runs = Arc::new(AtomicUsize::new(0));
    let path = Path::new("/footage/clip.mp4");
    let written = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let before = InputKey::for_file("cutout 1", path, 1024, Some(written));

    assert_eq!(
        outcome(cutout(&scheduler, &before, &runs, false)),
        Outcome::Failed(CutoutError::Offline)
    );
    // The same file, asked about again: the same key.
    assert_eq!(
        before,
        InputKey::for_file("cutout 1", path, 1024, Some(written))
    );
    // Saved over, grown, or read by a newer method: each is a new key,
    // and the job is tried afresh.
    let later = written + Duration::from_secs(60);
    for changed in [
        InputKey::for_file("cutout 1", path, 1024, Some(later)),
        InputKey::for_file("cutout 1", path, 2048, Some(written)),
        InputKey::for_file("cutout 2", path, 1024, Some(written)),
        InputKey::for_file(
            "cutout 1",
            Path::new("/footage/other.mp4"),
            1024,
            Some(written),
        ),
    ] {
        assert_ne!(changed, before);
        assert_eq!(
            outcome(cutout(&scheduler, &changed, &runs, true)),
            Outcome::Done("a matte")
        );
    }
    assert_eq!(runs.load(Ordering::Acquire), 5);
    // And the first is still remembered.
    assert!(scheduler.failed_before(&before));
}

#[test]
fn a_forgotten_failure_is_tried_again() {
    let scheduler = small(1);
    let runs = Arc::new(AtomicUsize::new(0));
    let clip = InputKey::new("clip");
    let other = InputKey::new("other");
    for input in [&clip, &other] {
        assert_eq!(
            outcome(cutout(&scheduler, input, &runs, false)),
            Outcome::Failed(CutoutError::Offline)
        );
    }

    // The person presses the button again.
    assert!(scheduler.forget(&clip));
    assert!(!scheduler.forget(&clip));
    assert_eq!(
        outcome(cutout(&scheduler, &clip, &runs, true)),
        Outcome::Done("a matte")
    );
    // It worked this time, and nothing is remembered of it.
    assert!(!scheduler.failed_before(&clip));
    assert!(scheduler.failed_before(&other));

    // The network comes back.
    scheduler.forget_all();
    assert_eq!(scheduler.snapshot().failures, 0);
    assert_eq!(
        outcome(cutout(&scheduler, &other, &runs, true)),
        Outcome::Done("a matte")
    );
    assert_eq!(runs.load(Ordering::Acquire), 4);
}

#[test]
fn only_a_failure_is_remembered_not_a_cancel_and_not_a_panic() {
    let scheduler = small(1);
    let clip = InputKey::new("clip");

    let (started, starts) = mpsc::channel();
    let cancelled = scheduler.submit(
        Job::new(Lane::Background, move |ctx| {
            started.send(()).expect("the test is listening");
            ctx.token().wait_timeout(BOUND);
            // What a cancelled download really returns.
            Err::<(), _>(JobError::Failed(CutoutError::Offline))
        })
        .input(clip.clone()),
    );
    next(&starts);
    cancelled.cancel();
    assert_eq!(
        outcome(cancelled),
        Outcome::Cancelled(CancelReason::Requested)
    );
    assert!(!scheduler.failed_before(&clip));

    let panicked = scheduler.submit(
        Job::<(), CutoutError>::fallible(Lane::Background, |_| {
            panic!("a panic on an input, for the test")
        })
        .input(clip.clone()),
    );
    assert!(matches!(outcome(panicked), Outcome::Panicked(_)));
    assert!(!scheduler.failed_before(&clip));
    assert_eq!(scheduler.snapshot().failures, 0);
}

#[test]
fn the_failures_remembered_are_bounded_and_the_oldest_goes_first() {
    let scheduler = Scheduler::new(Config::new(budget(1, [1; Lane::COUNT])).failure_memory(2));
    let runs = Arc::new(AtomicUsize::new(0));
    let keys: Vec<InputKey> = ["a", "b", "c"].into_iter().map(InputKey::new).collect();
    for key in &keys {
        assert_eq!(
            outcome(cutout(&scheduler, key, &runs, false)),
            Outcome::Failed(CutoutError::Offline)
        );
    }
    assert_eq!(scheduler.snapshot().failures, 2);
    assert!(!scheduler.failed_before(&keys[0]));
    assert!(scheduler.failed_before(&keys[1]));
    assert!(scheduler.failed_before(&keys[2]));
    // The one that was dropped is simply run again.
    assert_eq!(
        outcome(cutout(&scheduler, &keys[0], &runs, true)),
        Outcome::Done("a matte")
    );
}

#[test]
fn a_failure_of_one_kind_of_job_is_not_answered_to_another_kind() {
    let scheduler = small(1);
    let runs = Arc::new(AtomicUsize::new(0));
    let shared = InputKey::new("two kinds of job were given one key");
    assert_eq!(
        outcome(cutout(&scheduler, &shared, &runs, false)),
        Outcome::Failed(CutoutError::Offline)
    );
    // Its error is a different type, so the remembered one is not its
    // own; it is run.
    let other = scheduler.submit(
        Job::fallible(Lane::Background, |_| Ok::<_, String>("stabilized")).input(shared.clone()),
    );
    assert_eq!(outcome(other), Outcome::Done("stabilized"));
}

#[test]
fn a_job_that_panics_ends_as_panicked_and_its_thread_runs_the_next() {
    let (sink, seen) = events();
    // One thread on the lane: if the panic took it, nothing would follow.
    let scheduler = Scheduler::new(Config::new(budget(1, [1; Lane::COUNT])).events(sink));

    let text = scheduler.submit(Job::<(), ()>::fallible(Lane::Background, |_| {
        panic!("the model fell over")
    }));
    assert_eq!(
        outcome(text),
        Outcome::Panicked("the model fell over".to_owned())
    );
    let formatted = scheduler.submit(Job::<()>::infallible(Lane::Background, |ctx| {
        panic!("frame {} on the {} lane was bad", 12, ctx.lane())
    }));
    assert_eq!(
        outcome(formatted),
        Outcome::Panicked("frame 12 on the background lane was bad".to_owned())
    );
    let other = scheduler.submit(Job::<()>::infallible(Lane::Background, |_| {
        std::panic::panic_any(404u16)
    }));
    assert_eq!(
        outcome(other),
        Outcome::Panicked("a panic that carried no message".to_owned())
    );

    // The same thread, three panics later.
    let after = scheduler.submit(Job::infallible(Lane::Background, |_| {
        std::thread::current().name().map(str::to_owned)
    }));
    assert_eq!(
        outcome(after),
        Outcome::Done(Some("concat-background-0".to_owned()))
    );
    assert!(scheduler.snapshot().is_idle());

    drop(scheduler);
    let panics = seen
        .try_iter()
        .filter(|event| {
            matches!(
                event,
                Event::Finished {
                    end: End::Panicked(_),
                    ran: Some(_),
                    ..
                }
            )
        })
        .count();
    assert_eq!(panics, 3);
}

#[test]
fn a_callback_that_panics_does_not_take_its_thread_either() {
    let scheduler = small(1);
    let told = scheduler.submit_then(Job::infallible(Lane::Io, |_| 1), |_| {
        panic!("a callback that panics, for the test")
    });
    over(&told);
    assert_eq!(told.end(), Some(End::Done));
    let after = scheduler.submit(Job::infallible(Lane::Io, |_| 2));
    assert_eq!(outcome(after), Outcome::Done(2));
}

#[test]
fn an_events_function_that_panics_does_not_take_a_thread_either() {
    let scheduler = Scheduler::new(Config::new(budget(1, [1; Lane::COUNT])).events(|event| {
        if matches!(event, Event::Started(_)) {
            panic!("an events function that panics, for the test");
        }
    }));
    for round in 0..3 {
        let job = scheduler.submit(Job::infallible(Lane::Io, move |_| round));
        assert_eq!(outcome(job), Outcome::Done(round));
    }
}
