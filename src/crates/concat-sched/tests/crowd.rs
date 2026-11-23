// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Everything at once: jobs of every kind handed in from several threads
//! while others are cancelled, and what must be true whichever way the
//! threads fall.
//!
//! The other tests each put the scheduler in one known position. This
//! one does not know the position and does not need to: every job ends,
//! each in a way it could have ended, and when they have, nothing is
//! still held - no core, no resource, no thread.

mod common;

use std::sync::Arc;

use common::{BOUND, events, outcome};
use concat_sched::{
    CancelReason, CancelToken, Coalesce, Config, End, Event, Job, JobContext, JobError, JobHandle,
    Lane, Machine, Outcome, Owner, Resource, Scheduler, Ticket,
};

const SUBMITTERS: usize = 4;
const EACH: usize = 150;

/// The job numbered `index`, with whatever its number gives it: a lane,
/// an owner, a width, sometimes a key, a resource or an input, and now
/// and then a failure or a panic.
fn job(index: usize) -> Job<usize, usize> {
    let lane = Lane::ALL[index % Lane::COUNT];
    let mut job = Job::new(lane, move |ctx: &JobContext| {
        for step in 0..20u32 {
            ctx.checkpoint()?;
            ctx.progress(step as f32 / 20.0);
            std::thread::yield_now();
        }
        if index % 53 == 7 {
            panic!("job {index} panics, for the test");
        }
        if index % 11 == 3 {
            return Err(JobError::Failed(index));
        }
        Ok(index)
    })
    .label("one of the crowd")
    .owner(Owner((index % 3) as u64))
    .threads(1 + index % 5)
    .rank((index % 3) as u8);
    if index.is_multiple_of(4) {
        job = job.needs(Resource::GPU_MEMORY, 1 + (index % 3) as u64);
    }
    if index.is_multiple_of(5) {
        let coalesce = if index.is_multiple_of(10) {
            Coalesce::Replace
        } else {
            Coalesce::RunBoth
        };
        job = job.key(format!("key {}", index % 7), coalesce);
    }
    if index.is_multiple_of(9) {
        job = job.input(format!("input {}", index % 4));
    }
    job
}

/// Whether `outcome` is a way the job numbered `index` could have ended.
fn possible(index: usize, outcome: &Outcome<usize, usize>) -> bool {
    match outcome {
        Outcome::Done(value) => *value == index && index % 53 != 7 && index % 11 != 3,
        Outcome::Failed(error) => *error == index && index % 11 == 3,
        // Some earlier job on the same input, which failed.
        Outcome::FailedBefore(error) => {
            index.is_multiple_of(9)
                && error.is_multiple_of(9)
                && error % 11 == 3
                && error % 4 == index % 4
        }
        Outcome::Panicked(message) => {
            index % 53 == 7 && *message == format!("job {index} panics, for the test")
        }
        Outcome::Cancelled(CancelReason::Owner) => index % 3 == 1,
        Outcome::Cancelled(CancelReason::Replaced) => index.is_multiple_of(5),
        Outcome::Cancelled(CancelReason::Requested) => index % 13 == 5,
        // Nothing here shuts the scheduler down, fills a queue, asks for
        // more of a resource than there is or keeps the first of a key.
        Outcome::Cancelled(CancelReason::Shutdown) | Outcome::Refused(_) => false,
    }
}

#[test]
fn a_crowd_of_jobs_all_end_as_they_could_have_and_leave_nothing_held() {
    let (sink, seen) = events();
    // Room in every lane's line for all of them. With the usual limits a
    // slow machine has a hundred frames waiting where sixty-four fit, and
    // the rest are refused - which is the limit doing its work, and not
    // what is being tested here.
    let config = Lane::ALL.into_iter().fold(
        Config::for_machine(Machine::new(8, 16 << 30)).events(sink),
        |config, lane| config.queue_limit(lane, SUBMITTERS * EACH),
    );
    let scheduler = Scheduler::new(config);
    scheduler.set_capacity(Resource::GPU_MEMORY, 4);

    let handles: Vec<(usize, JobHandle<usize, usize>)> = std::thread::scope(|scope| {
        let submitters: Vec<_> = (0..SUBMITTERS)
            .map(|submitter| {
                let scheduler = &scheduler;
                scope.spawn(move || {
                    (0..EACH)
                        .map(|turn| {
                            let index = turn * SUBMITTERS + submitter;
                            let handle = scheduler.submit(job(index));
                            if index % 13 == 5 {
                                handle.cancel();
                            }
                            (index, handle)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        // Meanwhile a project closes, twice over, and the window looks on.
        for _ in 0..2 {
            let stopping = scheduler.cancel_owner(Owner(1));
            assert_eq!(stopping.wait(BOUND), Ok(()));
            let snapshot = scheduler.snapshot();
            assert!(
                snapshot
                    .lanes
                    .iter()
                    .all(|lane| lane.running.len() <= lane.workers)
            );
            let _ = scheduler.load();
        }
        submitters
            .into_iter()
            .flat_map(|submitter| submitter.join().expect("a submitter did not panic"))
            .collect()
    });
    assert_eq!(handles.len(), SUBMITTERS * EACH);

    for (index, handle) in handles {
        let ended = outcome(handle);
        assert!(possible(index, &ended), "job {index} ended as {ended:?}");
    }

    // Every job is over, so nothing is held by anyone.
    let snapshot = scheduler.snapshot();
    assert!(snapshot.is_idle(), "{snapshot:?}");
    assert_eq!(snapshot.cores, 0);
    assert!(snapshot.lanes.iter().all(|lane| lane.cores == 0));
    assert!(snapshot.resources.iter().all(|resource| resource.held == 0));
    assert!(scheduler.shutdown(BOUND).is_clean());

    // And the events tell the same story: one end for every job, and for
    // every start an end that says the job ran.
    let events: Vec<Event> = seen.try_iter().collect();
    let starts = events
        .iter()
        .filter(|event| matches!(event, Event::Started(_)))
        .count();
    let ends = events
        .iter()
        .filter(|event| matches!(event, Event::Finished { .. }))
        .count();
    let ran = events
        .iter()
        .filter(|event| matches!(event, Event::Finished { ran: Some(_), .. }))
        .count();
    let paused = events
        .iter()
        .filter(|event| matches!(event, Event::Paused(_)))
        .count();
    let resumed = events
        .iter()
        .filter(|event| matches!(event, Event::Resumed(_)))
        .count();
    assert_eq!(ends, SUBMITTERS * EACH);
    assert_eq!(starts, ran);
    assert!(resumed <= paused);
    assert!(events.iter().all(|event| !matches!(
        event,
        Event::Finished {
            end: End::Refused(_),
            ..
        }
    )));
}

#[test]
fn the_scheduler_and_the_hands_on_a_job_cross_threads() {
    fn crosses<T: Send + Sync>() {}
    fn moves<T: Send>() {}
    crosses::<Scheduler>();
    crosses::<Arc<Scheduler>>();
    crosses::<CancelToken>();
    crosses::<Ticket>();
    // A job may hand its context to threads of its own, to check from.
    crosses::<JobContext>();
    moves::<JobHandle<Vec<u8>, String>>();
    moves::<Job<Vec<u8>, String>>();
}
