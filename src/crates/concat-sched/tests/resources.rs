// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Resources: a job waits until what it needs is free, and gives it back
//! however it ends.

mod common;

use std::sync::mpsc;

use common::{BOUND, Gate, budget, log, named, next, outcome, state_of, written};
use concat_sched::{
    CancelReason, Config, Job, JobError, JobHandle, JobState, Lane, Outcome, Refusal, Resource,
    ResourceUse, Scheduler,
};

const ENCODERS: Resource = Resource::ENCODER_SESSIONS;
const GPU: Resource = Resource::GPU_MEMORY;

/// A scheduler with room for three foreground jobs at once, so that only
/// a resource can be what keeps one waiting.
fn roomy() -> Scheduler {
    Scheduler::new(Config::new(budget(8, [1, 1, 3, 1, 3, 1])))
}

fn held(scheduler: &Scheduler, resource: Resource) -> u64 {
    scheduler
        .snapshot()
        .resources
        .iter()
        .find(|used| used.resource == resource)
        .map_or(0, |used| used.held)
}

/// Starts a job that holds `amount` of `resource` and waits at `gate`;
/// returns once it is running.
fn holding(scheduler: &Scheduler, resource: Resource, amount: u64, gate: &Gate) -> JobHandle<()> {
    let (started, starts) = mpsc::channel();
    let gate = gate.clone();
    let job = scheduler.submit(
        Job::infallible(Lane::Foreground, move |_| {
            started.send(()).expect("the test is listening");
            gate.pass();
        })
        .needs(resource, amount),
    );
    next(&starts);
    job
}

#[test]
fn a_job_waits_for_its_resource_and_others_go_round_it() {
    let scheduler = roomy();
    scheduler.set_capacity(ENCODERS, 1);
    let gate = Gate::new();
    let first = holding(&scheduler, ENCODERS, 1, &gate);
    assert_eq!(held(&scheduler, ENCODERS), 1);

    let log = log();
    let second = scheduler.submit(named(Lane::Foreground, "second", &log).needs(ENCODERS, 1));
    // Two threads are idle and the cores are there; the session is not.
    assert_eq!(state_of(&scheduler, &second), Some(JobState::Queued));

    // A job that needs no session is not kept behind the one that does.
    let other = scheduler.submit(named(Lane::Foreground, "other", &log));
    assert_eq!(outcome(other), Outcome::Done(()));
    assert_eq!(state_of(&scheduler, &second), Some(JobState::Queued));

    gate.open();
    assert_eq!(outcome(first), Outcome::Done(()));
    assert_eq!(outcome(second), Outcome::Done(()));
    assert_eq!(written(&log), ["other", "second"]);
    assert_eq!(held(&scheduler, ENCODERS), 0);
}

#[test]
fn a_job_that_panics_gives_back_what_it_held() {
    let scheduler = roomy();
    scheduler.set_capacity(GPU, 100);
    let gate = Gate::new();
    let (started, starts) = mpsc::channel();
    let doomed = {
        let gate = gate.clone();
        scheduler.submit(
            Job::infallible(Lane::Foreground, move |_| {
                started.send(()).expect("the test is listening");
                gate.pass();
                panic!("a job that panics while holding memory, for the test");
            })
            .needs(GPU, 80),
        )
    };
    next(&starts);
    assert_eq!(held(&scheduler, GPU), 80);
    let next_one = scheduler.submit(Job::infallible(Lane::Foreground, |_| "ran").needs(GPU, 60));
    assert_eq!(state_of(&scheduler, &next_one), Some(JobState::Queued));

    gate.open();
    assert!(matches!(outcome(doomed), Outcome::Panicked(_)));
    assert_eq!(outcome(next_one), Outcome::Done("ran"));
    assert_eq!(held(&scheduler, GPU), 0);
}

#[test]
fn a_job_that_is_cancelled_gives_back_what_it_held() {
    let scheduler = roomy();
    scheduler.set_capacity(GPU, 100);
    let (started, starts) = mpsc::channel();
    let running = scheduler.submit(
        Job::new(Lane::Foreground, move |ctx| {
            started.send(()).expect("the test is listening");
            ctx.token().wait_timeout(BOUND);
            ctx.checkpoint()?;
            Ok::<_, JobError<()>>(())
        })
        .needs(GPU, 100),
    );
    next(&starts);
    let waiting = scheduler.submit(Job::infallible(Lane::Foreground, |_| "ran").needs(GPU, 100));
    // A job cancelled while it waits held nothing, and takes nothing.
    let never = scheduler.submit(Job::infallible(Lane::Foreground, |_| "never").needs(GPU, 100));
    never.cancel();
    assert_eq!(outcome(never), Outcome::Cancelled(CancelReason::Requested));
    assert_eq!(held(&scheduler, GPU), 100);

    running.cancel();
    assert_eq!(
        outcome(running),
        Outcome::Cancelled(CancelReason::Requested)
    );
    assert_eq!(outcome(waiting), Outcome::Done("ran"));
    assert_eq!(held(&scheduler, GPU), 0);
}

#[test]
fn a_job_that_fails_gives_back_what_it_held() {
    let scheduler = roomy();
    scheduler.set_capacity(ENCODERS, 2);
    let failed = scheduler.submit(
        Job::fallible(Lane::Foreground, |_| Err::<(), _>("no such encoder")).needs(ENCODERS, 2),
    );
    assert_eq!(outcome(failed), Outcome::Failed("no such encoder"));
    assert_eq!(held(&scheduler, ENCODERS), 0);
}

#[test]
fn a_job_waiting_for_a_resource_is_not_passed_by_a_later_job_that_needs_less() {
    let scheduler = roomy();
    scheduler.set_capacity(GPU, 2);
    let gate = Gate::new();
    let first = holding(&scheduler, GPU, 1, &gate);

    let log = log();
    let large = scheduler.submit(named(Lane::Foreground, "large", &log).needs(GPU, 2));
    let small = scheduler.submit(named(Lane::Foreground, "small", &log).needs(GPU, 1));
    // One unit is free and the small job would fit - and if it took it,
    // and the next small job after it, the large one would wait for ever.
    assert_eq!(held(&scheduler, GPU), 1);
    assert_eq!(state_of(&scheduler, &large), Some(JobState::Queued));
    assert_eq!(state_of(&scheduler, &small), Some(JobState::Queued));
    // The same holds for a job on a lane below.
    let below = scheduler.submit(named(Lane::Background, "below", &log).needs(GPU, 1));
    assert_eq!(state_of(&scheduler, &below), Some(JobState::Queued));

    gate.open();
    assert_eq!(outcome(first), Outcome::Done(()));
    for job in [large, small, below] {
        assert_eq!(outcome(job), Outcome::Done(()));
    }
    assert_eq!(written(&log)[0], "large");
}

#[test]
fn a_job_that_needs_more_than_there_is_is_refused_at_once() {
    let scheduler = roomy();
    scheduler.set_capacity(GPU, 100);
    let greedy = scheduler.submit(Job::infallible(Lane::Foreground, |_| ()).needs(GPU, 101));
    assert!(greedy.is_finished());
    assert_eq!(outcome(greedy), Outcome::Refused(Refusal::TooLarge(GPU)));

    // A resource nobody has declared has nothing in it.
    let unknown = Resource::new("licences");
    let hopeful = scheduler.submit(Job::infallible(Lane::Foreground, |_| ()).needs(unknown, 1));
    assert_eq!(
        outcome(hopeful),
        Outcome::Refused(Refusal::TooLarge(unknown))
    );
    // Needing none of it is needing nothing.
    let modest = scheduler.submit(Job::infallible(Lane::Foreground, |_| "ran").needs(unknown, 0));
    assert_eq!(outcome(modest), Outcome::Done("ran"));
}

#[test]
fn more_of_a_resource_starts_the_job_that_was_waiting_for_it() {
    let scheduler = roomy();
    scheduler.set_capacity(GPU, 4);
    let gate = Gate::new();
    let first = holding(&scheduler, GPU, 3, &gate);
    let second = scheduler.submit(Job::infallible(Lane::Foreground, |_| "ran").needs(GPU, 3));
    assert_eq!(state_of(&scheduler, &second), Some(JobState::Queued));

    scheduler.set_capacity(GPU, 6);
    assert_eq!(outcome(second), Outcome::Done("ran"));

    // Less than is held takes nothing from the job holding it.
    scheduler.set_capacity(GPU, 1);
    assert_eq!(state_of(&scheduler, &first), Some(JobState::Running));
    assert_eq!(
        scheduler.snapshot().resources,
        [ResourceUse {
            resource: GPU,
            capacity: 1,
            held: 3
        }]
    );
    gate.open();
    assert_eq!(outcome(first), Outcome::Done(()));
    assert_eq!(held(&scheduler, GPU), 0);
}

#[test]
fn a_job_holds_every_resource_it_asked_for_or_none() {
    let scheduler = roomy();
    scheduler.set_capacity(GPU, 10);
    scheduler.set_capacity(ENCODERS, 1);
    let gate = Gate::new();
    let encoding = holding(&scheduler, ENCODERS, 1, &gate);

    // The memory is free and the session is not: it takes neither.
    let both = scheduler.submit(
        Job::infallible(Lane::Foreground, |_| "ran")
            .needs(GPU, 4)
            .needs(GPU, 4)
            .needs(ENCODERS, 1),
    );
    assert_eq!(state_of(&scheduler, &both), Some(JobState::Queued));
    assert_eq!(held(&scheduler, GPU), 0);

    gate.open();
    assert_eq!(outcome(encoding), Outcome::Done(()));
    assert_eq!(outcome(both), Outcome::Done("ran"));
    assert_eq!(held(&scheduler, GPU), 0);
    assert_eq!(held(&scheduler, ENCODERS), 0);
}
