// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! What an observer sees: the snapshot, a job's progress, and the events.

mod common;

use std::sync::mpsc;

use common::{BOUND, Gate, budget, events, hold, next, outcome, over, small};
use concat_sched::{
    CancelReason, Coalesce, Config, End, Event, Job, JobError, JobKey, JobState, Lane, Outcome,
    Owner, Progress, Refusal, Scheduler,
};

#[test]
fn a_snapshot_shows_what_runs_and_what_waits_on_each_lane() {
    let scheduler = small(2);
    assert!(scheduler.snapshot().is_idle());

    let (started, starts) = mpsc::channel();
    let gate = Gate::new();
    let export = {
        let gate = gate.clone();
        scheduler.submit(
            Job::infallible(Lane::Foreground, move |ctx| {
                ctx.stage("export.picture");
                ctx.progress(0.25);
                started.send(()).expect("the test is listening");
                gate.pass();
            })
            .label("export")
            .owner(Owner(4))
            .key("export", Coalesce::KeepFirst)
            .wide(),
        )
    };
    next(&starts);
    let merge = scheduler.submit(Job::infallible(Lane::Foreground, |_| ()).label("merge"));

    let snapshot = scheduler.snapshot();
    assert!(!snapshot.is_idle());
    assert_eq!(snapshot.pool, 2);
    assert_eq!(snapshot.cores, 2);
    assert_eq!(snapshot.jobs().count(), 2);

    let lane = snapshot.lane(Lane::Foreground);
    assert_eq!(
        (lane.lane, lane.workers, lane.cores),
        (Lane::Foreground, 1, 2)
    );
    assert_eq!(lane.running.len(), 1);
    let running = &lane.running[0];
    assert_eq!(running.id, export.id());
    assert_eq!(running.lane, Lane::Foreground);
    assert_eq!(running.label, "export");
    assert_eq!(running.owner, Some(Owner(4)));
    assert_eq!(running.key, Some(JobKey::new("export")));
    assert_eq!(running.state, JobState::Running);
    assert_eq!(
        running.progress,
        Progress {
            fraction: 0.25,
            stage: Some("export.picture")
        }
    );
    assert_eq!(running.threads, 2);
    // It has been running for no longer than it has existed.
    let running_for = running.running_for.expect("it has started");
    assert!(running_for <= running.age);

    assert_eq!(lane.queued.len(), 1);
    let queued = &lane.queued[0];
    assert_eq!(queued.id, merge.id());
    assert_eq!(queued.label, "merge");
    assert_eq!(queued.owner, None);
    assert_eq!(queued.key, None);
    assert_eq!(queued.state, JobState::Queued);
    assert_eq!(queued.progress, Progress::default());
    assert_eq!(queued.threads, 0);
    assert_eq!(queued.running_for, None);
    assert_eq!(snapshot.job(merge.id()), Some(queued));

    for other in [
        Lane::Frame,
        Lane::Save,
        Lane::Prefetch,
        Lane::Background,
        Lane::Io,
    ] {
        let lane = snapshot.lane(other);
        assert_eq!(lane.lane, other);
        assert!(lane.running.is_empty() && lane.queued.is_empty());
    }

    gate.open();
    assert_eq!(outcome(export), Outcome::Done(()));
    assert_eq!(outcome(merge), Outcome::Done(()));
    assert!(scheduler.snapshot().is_idle());
    assert_eq!(scheduler.snapshot().cores, 0);
}

#[test]
fn a_job_handed_in_earlier_is_the_older_of_two() {
    let scheduler = small(1);
    let gate = Gate::new();
    let holder = hold(&scheduler, Lane::Io, &gate);
    let earlier = scheduler.submit(Job::infallible(Lane::Io, |_| ()));
    let later = scheduler.submit(Job::infallible(Lane::Io, |_| ()));
    assert!(earlier.id() < later.id());
    // The later one is asked first, so the earlier one's answer is of a
    // later moment as well as an earlier start.
    let younger = later.info().age;
    let older = earlier.info().age;
    assert!(older >= younger);
    assert!(holder.info().age >= older);
    assert_eq!(earlier.info().running_for, None);
    gate.open();
    for job in [holder, earlier, later] {
        assert_eq!(outcome(job), Outcome::Done(()));
    }
}

#[test]
fn a_jobs_progress_can_be_read_from_its_ticket_while_it_runs() {
    let scheduler = small(1);
    let (reached, reaches) = mpsc::channel();
    let (go, goes) = mpsc::channel::<()>();
    let job = scheduler.submit(Job::new(Lane::Background, move |ctx| {
        for (fraction, stage) in [(0.5, "captions.listening"), (1.5, "captions.placing")] {
            ctx.checkpoint()?;
            ctx.stage(stage);
            ctx.progress(fraction);
            // Not a number, and so not said.
            ctx.progress(f32::NAN);
            reached.send(()).expect("the test is listening");
            goes.recv_timeout(BOUND).expect("the test says when");
        }
        Ok::<_, JobError<()>>(())
    }));
    let ticket = job.ticket();
    assert_eq!(ticket.id(), job.id());
    assert_eq!(ticket.lane(), Lane::Background);

    next(&reaches);
    assert_eq!(ticket.state(), JobState::Running);
    assert_eq!(
        ticket.progress(),
        Progress {
            fraction: 0.5,
            stage: Some("captions.listening")
        }
    );
    assert_eq!(ticket.fraction(), 0.5);
    go.send(()).expect("the job is waiting");
    next(&reaches);
    // Past the end is the end.
    assert_eq!(
        ticket.progress(),
        Progress {
            fraction: 1.0,
            stage: Some("captions.placing")
        }
    );
    go.send(()).expect("the job is waiting");
    assert_eq!(outcome(job), Outcome::Done(()));
    // What it last said stays readable after it is over.
    assert_eq!(ticket.progress().fraction, 1.0);
    assert_eq!(ticket.state(), JobState::Ended);
    assert_eq!(ticket.end(), Some(End::Done));
}

#[test]
fn the_events_of_a_job_come_in_order_and_say_how_it_ended() {
    let (sink, seen) = events();
    let scheduler = Scheduler::new(Config::new(budget(1, [1; Lane::COUNT])).events(sink));

    // One that finishes.
    let done = scheduler.submit(
        Job::infallible(Lane::Io, |_| ())
            .label("probe")
            .owner(Owner(2)),
    );
    let done_id = done.id();
    assert_eq!(outcome(done), Outcome::Done(()));
    let started = next(&seen);
    let Event::Started(job) = &started else {
        panic!("the first event was {started:?}");
    };
    assert_eq!(
        (job.id, job.label, job.state),
        (done_id, "probe", JobState::Running)
    );
    assert_eq!(job.owner, Some(Owner(2)));
    assert_eq!(started.job().id, done_id);
    let finished = next(&seen);
    let Event::Finished { job, end, ran } = &finished else {
        panic!("the second event was {finished:?}");
    };
    assert_eq!((job.id, job.state), (done_id, JobState::Ended));
    assert_eq!(end, &End::Done);
    assert!(ran.is_some());

    // One that fails.
    let failed = scheduler.submit(Job::fallible(Lane::Io, |_| Err::<(), _>("no such file")));
    let failed_id = failed.id();
    assert_eq!(outcome(failed), Outcome::Failed("no such file"));
    assert!(matches!(next(&seen), Event::Started(job) if job.id == failed_id));
    assert!(matches!(
        next(&seen),
        Event::Finished { job, end: End::Failed, ran: Some(_) } if job.id == failed_id
    ));

    // One cancelled while it waits: an end, and no start.
    let gate = Gate::new();
    let holder = hold(&scheduler, Lane::Io, &gate);
    assert!(matches!(next(&seen), Event::Started(job) if job.id == holder.id()));
    let waiting = scheduler.submit(Job::infallible(Lane::Io, |_| ()));
    waiting.cancel();
    assert!(matches!(
        next(&seen),
        Event::Finished {
            job,
            end: End::Cancelled(CancelReason::Requested),
            ran: None
        } if job.id == waiting.id()
    ));

    // One refused: the same.
    let refused =
        scheduler.submit(Job::infallible(Lane::Io, |_| ()).key("probe", Coalesce::KeepFirst));
    let twice =
        scheduler.submit(Job::infallible(Lane::Io, |_| ()).key("probe", Coalesce::KeepFirst));
    assert!(matches!(
        next(&seen),
        Event::Finished {
            job,
            end: End::Refused(Refusal::Duplicate(first)),
            ran: None
        } if job.id == twice.id() && first == refused.id()
    ));

    gate.open();
    over(&holder);
    over(&refused);
    drop(scheduler);
    // Every job that started also finished, and nothing was said twice.
    let rest: Vec<Event> = seen.try_iter().collect();
    let starts = rest
        .iter()
        .filter(|event| matches!(event, Event::Started(_)))
        .count();
    let ends = rest
        .iter()
        .filter(|event| matches!(event, Event::Finished { .. }))
        .count();
    assert_eq!((starts, ends), (1, 2));
}

#[test]
fn the_events_function_may_ask_the_scheduler_what_it_likes() {
    // It is called with no lock held; if it were, this would never end.
    let (asked, answers) = mpsc::channel();
    let scheduler = std::sync::Arc::new(std::sync::OnceLock::<Scheduler>::new());
    let inside = std::sync::Arc::clone(&scheduler);
    let asked = std::sync::Mutex::new(asked);
    let made = Scheduler::new(
        Config::new(budget(1, [1; Lane::COUNT])).events(move |event| {
            if let (Event::Started(_), Some(scheduler)) = (event, inside.get()) {
                let running = scheduler.snapshot().jobs().count();
                let load = scheduler.load()[Lane::Io.index()].running;
                asked
                    .lock()
                    .expect("the sender's lock")
                    .send((running, load))
                    .expect("the test is listening");
            }
        }),
    );
    assert!(scheduler.set(made).is_ok());
    let scheduler = scheduler.get().expect("it was just set");
    let job = scheduler.submit(Job::infallible(Lane::Io, |_| ()));
    assert_eq!(next(&answers), (1, 1));
    assert_eq!(outcome(job), Outcome::Done(()));
    assert!(scheduler.shutdown(BOUND).is_clean());
}
