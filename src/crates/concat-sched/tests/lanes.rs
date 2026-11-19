// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The lanes: what starts first, who waits for whom, and how the cores
//! are lent and given back.

mod common;

use std::sync::mpsc;

use common::{
    BOUND, Gate, budget, events, hold, hold_wide, log, named, next, outcome, small, state_of,
    written,
};
use concat_sched::{
    Budget, Config, Event, Job, JobError, JobHandle, JobState, Lane, Machine, Outcome, Refusal,
    Scheduler,
};

#[test]
fn jobs_waiting_on_different_lanes_start_most_urgent_first() {
    let scheduler = small(1);
    let gate = Gate::new();
    // The pool's one core is taken, so everything after this waits.
    let holder = hold(&scheduler, Lane::Foreground, &gate);

    let log = log();
    let background = scheduler.submit(named(Lane::Background, "background", &log));
    let prefetch = scheduler.submit(named(Lane::Prefetch, "prefetch", &log));
    let foreground = scheduler.submit(named(Lane::Foreground, "foreground", &log));
    for waiting in [&background, &prefetch, &foreground] {
        assert_eq!(state_of(&scheduler, waiting), Some(JobState::Queued));
    }

    gate.open();
    for job in [holder, background, prefetch, foreground] {
        assert_eq!(outcome(job), Outcome::Done(()));
    }
    assert_eq!(written(&log), ["foreground", "prefetch", "background"]);
}

#[test]
fn jobs_waiting_on_one_lane_start_by_rank_and_then_by_age() {
    let scheduler = small(1);
    let gate = Gate::new();
    let holder = hold(&scheduler, Lane::Background, &gate);

    let log = log();
    let jobs = [
        scheduler.submit(named(Lane::Background, "last", &log).rank(2)),
        scheduler.submit(named(Lane::Background, "first", &log)),
        scheduler.submit(named(Lane::Background, "second", &log)),
        scheduler.submit(named(Lane::Background, "third", &log).rank(1)),
    ];
    let queued: Vec<&str> = scheduler
        .snapshot()
        .lane(Lane::Background)
        .queued
        .iter()
        .map(|job| job.label)
        .collect();
    assert_eq!(queued, ["first", "second", "third", "last"]);

    gate.open();
    assert_eq!(outcome(holder), Outcome::Done(()));
    for job in jobs {
        assert_eq!(outcome(job), Outcome::Done(()));
    }
    assert_eq!(written(&log), ["first", "second", "third", "last"]);
}

#[test]
fn the_frame_and_the_save_start_at_once_whatever_else_is_running() {
    // What used to freeze the picture and hold the autosave for minutes:
    // every thread busy with a long job, and more of them waiting.
    let scheduler = small(2);
    let gate = Gate::new();
    let (export, cores) = hold_wide(&scheduler, Lane::Foreground, &gate, usize::MAX);
    assert_eq!(cores, 2);
    let download = hold(&scheduler, Lane::Io, &gate);
    let log = log();
    let analysis = scheduler.submit(named(Lane::Background, "analysis", &log));
    let decode = scheduler.submit(named(Lane::Prefetch, "decode", &log));
    assert_eq!(state_of(&scheduler, &analysis), Some(JobState::Queued));
    assert_eq!(state_of(&scheduler, &decode), Some(JobState::Queued));

    let frame = scheduler.submit(Job::infallible(Lane::Frame, |_| "frame"));
    let save = scheduler.submit(Job::infallible(Lane::Save, |_| "saved"));
    assert_eq!(outcome(frame), Outcome::Done("frame"));
    assert_eq!(outcome(save), Outcome::Done("saved"));

    // And nothing else has moved meanwhile.
    assert_eq!(state_of(&scheduler, &export), Some(JobState::Running));
    assert_eq!(state_of(&scheduler, &download), Some(JobState::Running));
    assert_eq!(state_of(&scheduler, &analysis), Some(JobState::Queued));
    assert!(written(&log).is_empty());

    gate.open();
    for job in [export, download, analysis, decode] {
        assert_eq!(outcome(job), Outcome::Done(()));
    }
}

#[test]
fn a_lane_never_waits_for_a_lane_below_it() {
    let scheduler = small(4);
    let below = Gate::new();
    let above = Gate::new();
    // An analysis on the whole pool that never comes to a checkpoint.
    let (analysis, cores) = hold_wide(&scheduler, Lane::Background, &below, usize::MAX);
    assert_eq!(cores, 4);

    // The export starts all the same, on everything it is due.
    let (export, cores) = hold_wide(&scheduler, Lane::Foreground, &above, usize::MAX);
    assert_eq!(cores, 4);
    let snapshot = scheduler.snapshot();
    assert_eq!(state_of(&scheduler, &analysis), Some(JobState::Running));
    // The machine is over-full until the analysis ends, and says so.
    assert_eq!(snapshot.pool, 4);
    assert_eq!(snapshot.cores, 8);

    above.open();
    assert_eq!(outcome(export), Outcome::Done(()));
    below.open();
    assert_eq!(outcome(analysis), Outcome::Done(()));
}

#[test]
fn a_job_on_lent_cores_stands_aside_at_its_next_checkpoint_and_comes_back() {
    let (sink, seen) = events();
    let scheduler = Scheduler::new(Config::new(budget(4, [1; Lane::COUNT])).events(sink));
    let (started, starts) = mpsc::channel();
    let (go, goes) = mpsc::channel::<()>();
    let (checked, checks) = mpsc::channel();
    let analysis = scheduler.submit(
        Job::new(Lane::Background, move |ctx| {
            started.send(ctx.threads()).expect("the test is listening");
            goes.recv_timeout(BOUND).expect("the test says when");
            let held = ctx.checkpoint();
            checked.send(held.is_ok()).expect("the test is listening");
            held?;
            Ok::<_, JobError<()>>(ctx.threads())
        })
        .label("analysis")
        .wide(),
    );
    // Nothing above it is running, so it has the whole pool.
    assert_eq!(next(&starts), 4);
    assert!(matches!(next(&seen), Event::Started(job) if job.id == analysis.id()));

    let gate = Gate::new();
    let (export, cores) = hold_wide(&scheduler, Lane::Foreground, &gate, usize::MAX);
    assert_eq!(cores, 4);
    assert!(matches!(next(&seen), Event::Started(job) if job.id == export.id()));
    // Asked, but it has not reached a checkpoint yet.
    assert_eq!(state_of(&scheduler, &analysis), Some(JobState::Running));
    assert_eq!(scheduler.snapshot().cores, 8);

    go.send(()).expect("the job is waiting");
    assert!(matches!(next(&seen), Event::Paused(job) if job.id == analysis.id()));
    let snapshot = scheduler.snapshot();
    assert_eq!(state_of(&scheduler, &analysis), Some(JobState::Paused));
    assert_eq!(snapshot.lane(Lane::Background).cores, 0);
    assert_eq!(snapshot.cores, 4);
    // Held: its checkpoint has not returned.
    assert!(checks.try_recv().is_err());

    gate.open();
    let export_id = export.id();
    assert_eq!(outcome(export), Outcome::Done(()));
    // The export's end and the analysis's return are said by two
    // threads, in either order.
    let after = [next(&seen), next(&seen)];
    assert!(
        after
            .iter()
            .any(|event| matches!(event, Event::Finished { job, .. } if job.id == export_id))
    );
    assert!(
        after
            .iter()
            .any(|event| matches!(event, Event::Resumed(job) if job.id == analysis.id()))
    );
    assert!(next(&checks));
    // It comes back to the cores it had.
    assert_eq!(outcome(analysis), Outcome::Done(4));
}

#[test]
fn a_job_is_not_held_when_the_lane_above_takes_only_what_was_free() {
    let scheduler = small(4);
    let (started, starts) = mpsc::channel();
    let (go, goes) = mpsc::channel::<()>();
    let (checked, checks) = mpsc::channel();
    let analysis = scheduler.submit(
        Job::new(Lane::Background, move |ctx| {
            started.send(ctx.threads()).expect("the test is listening");
            goes.recv_timeout(BOUND).expect("the test says when");
            ctx.checkpoint()?;
            checked.send(()).expect("the test is listening");
            Ok::<_, JobError<()>>(())
        })
        .threads(2),
    );
    assert_eq!(next(&starts), 2);

    let gate = Gate::new();
    let (export, cores) = hold_wide(&scheduler, Lane::Foreground, &gate, 2);
    assert_eq!(cores, 2);
    assert_eq!(scheduler.snapshot().cores, 4);

    // The export is still running, and the checkpoint goes straight
    // through: two and two are the pool.
    go.send(()).expect("the job is waiting");
    next(&checks);
    assert_eq!(outcome(analysis), Outcome::Done(()));
    assert_eq!(state_of(&scheduler, &export), Some(JobState::Running));
    gate.open();
    assert_eq!(outcome(export), Outcome::Done(()));
}

#[test]
fn a_job_above_that_has_come_and_gone_holds_nothing_below() {
    let (sink, seen) = events();
    let scheduler = Scheduler::new(Config::new(budget(4, [1; Lane::COUNT])).events(sink));
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

    // A short job above, on the same cores, over before the analysis
    // looks: it was asked to stand aside, and is no longer.
    let decode = scheduler.submit(Job::infallible(Lane::Prefetch, |_| ()));
    assert_eq!(outcome(decode), Outcome::Done(()));
    go.send(()).expect("the job is waiting");
    assert_eq!(outcome(analysis), Outcome::Done(()));

    drop(scheduler);
    let paused = seen
        .try_iter()
        .filter(|event| matches!(event, Event::Paused(_)))
        .count();
    assert_eq!(paused, 0);
}

#[test]
fn a_job_alone_on_an_idle_machine_has_all_its_lane_may_have() {
    let machine = Machine::new(16, 64 << 30);
    let scheduler = Scheduler::new(Config::for_machine(machine));
    let budget = Budget::for_machine(machine);
    assert_eq!(scheduler.budget(), &budget);

    let analysis = scheduler.submit(Job::infallible(Lane::Background, |ctx| ctx.threads()).wide());
    assert_eq!(outcome(analysis), Outcome::Done(15));
    let export = scheduler.submit(Job::infallible(Lane::Foreground, |ctx| ctx.threads()).wide());
    assert_eq!(outcome(export), Outcome::Done(13));
    // A job that does not say how wide it is has one core.
    let narrow = scheduler.submit(Job::infallible(Lane::Background, |ctx| ctx.threads()));
    assert_eq!(outcome(narrow), Outcome::Done(1));
    // A job is granted no more than it can use.
    let some = scheduler.submit(Job::infallible(Lane::Background, |ctx| ctx.threads()).threads(6));
    assert_eq!(outcome(some), Outcome::Done(6));
}

#[test]
fn a_lane_below_runs_on_what_the_lane_above_leaves() {
    let scheduler = small(4);
    let gate = Gate::new();
    let (export, cores) = hold_wide(&scheduler, Lane::Foreground, &gate, 3);
    assert_eq!(cores, 3);
    // Wide, but one core is what there is, and it is worth starting on.
    let (analysis, cores) = hold_wide(&scheduler, Lane::Background, &gate, usize::MAX);
    assert_eq!(cores, 1);
    assert_eq!(scheduler.snapshot().cores, 4);
    gate.open();
    assert_eq!(outcome(export), Outcome::Done(()));
    assert_eq!(outcome(analysis), Outcome::Done(()));
}

/// An analysis on the whole of a four-core pool that reports its start on
/// the first channel, waits to be told on the second, and then comes to a
/// checkpoint; it writes "analysis" in `log` as it ends.
fn analysis_of_four(
    scheduler: &Scheduler,
    log: &std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>,
) -> (JobHandle<(), ()>, mpsc::Sender<()>) {
    let (started, starts) = mpsc::channel();
    let (go, goes) = mpsc::channel::<()>();
    let log = std::sync::Arc::clone(log);
    let analysis = scheduler.submit(
        Job::new(Lane::Background, move |ctx| {
            started.send(ctx.threads()).expect("the test is listening");
            goes.recv_timeout(BOUND).expect("the test says when");
            ctx.checkpoint()?;
            log.lock().expect("the log's lock").push("analysis");
            Ok::<_, JobError<()>>(())
        })
        .label("analysis")
        .wide(),
    );
    assert_eq!(next(&starts), 4);
    (analysis, go)
}

#[test]
fn a_wide_job_is_not_held_for_the_sake_of_one_core() {
    let scheduler = small(4);
    let log = log();
    let (analysis, go) = analysis_of_four(&scheduler, &log);
    let gate = Gate::new();
    let (small_job, cores) = hold_wide(&scheduler, Lane::Foreground, &gate, 1);
    assert_eq!(cores, 1);
    // One core over. Holding the analysis would idle three to free it.
    assert_eq!(scheduler.snapshot().cores, 5);

    go.send(()).expect("the job is waiting");
    assert_eq!(outcome(analysis), Outcome::Done(()));
    assert_eq!(state_of(&scheduler, &small_job), Some(JobState::Running));
    gate.open();
    assert_eq!(outcome(small_job), Outcome::Done(()));
}

#[test]
fn a_job_is_held_when_half_of_what_it_holds_is_wanted_above() {
    let (sink, seen) = events();
    let scheduler = Scheduler::new(Config::new(budget(4, [1; Lane::COUNT])).events(sink));
    let log = log();
    let (analysis, go) = analysis_of_four(&scheduler, &log);
    let gate = Gate::new();
    let (export, cores) = hold_wide(&scheduler, Lane::Foreground, &gate, 2);
    assert_eq!(cores, 2);

    go.send(()).expect("the job is waiting");
    loop {
        if matches!(next(&seen), Event::Paused(job) if job.id == analysis.id()) {
            break;
        }
    }
    assert_eq!(scheduler.snapshot().cores, 2);
    assert!(written(&log).is_empty());

    gate.open();
    assert_eq!(outcome(export), Outcome::Done(()));
    assert_eq!(outcome(analysis), Outcome::Done(()));
    assert_eq!(written(&log), ["analysis"]);
}

#[test]
fn a_job_coming_back_is_not_kept_out_by_younger_jobs_of_its_own_lane() {
    let (sink, seen) = events();
    let mut budget = budget(4, [1, 1, 1, 1, 3, 1]);
    budget.lanes[Lane::Background.index()].reserve = 1;
    let scheduler = Scheduler::new(Config::new(budget).events(sink));
    let log = log();
    let (analysis, go) = analysis_of_four(&scheduler, &log);

    // An export takes all but the core kept for the lane below, and the
    // analysis stands aside for it.
    let above = Gate::new();
    let (export, cores) = hold_wide(&scheduler, Lane::Foreground, &above, usize::MAX);
    assert_eq!(cores, 3);
    go.send(()).expect("the job is waiting");
    loop {
        if matches!(next(&seen), Event::Paused(job) if job.id == analysis.id()) {
            break;
        }
    }

    // While the export runs, a small job has the core that is kept.
    let beside = Gate::new();
    let small_job = hold(&scheduler, Lane::Background, &beside);
    assert_eq!(scheduler.snapshot().cores, 4);

    // The export ends. Three cores are free and the analysis wants four:
    // all that is in its way is the small job, which came after it.
    above.open();
    assert_eq!(outcome(export), Outcome::Done(()));
    assert_eq!(state_of(&scheduler, &analysis), Some(JobState::Paused));
    // So nothing more is started on the lane, cores and a thread to
    // spare or not, or a run of small jobs would keep it out for good.
    let later = scheduler.submit(named(Lane::Background, "later", &log));
    assert_eq!(state_of(&scheduler, &later), Some(JobState::Queued));
    assert_eq!(scheduler.snapshot().cores, 1);

    beside.open();
    assert_eq!(outcome(small_job), Outcome::Done(()));
    assert_eq!(outcome(analysis), Outcome::Done(()));
    assert_eq!(outcome(later), Outcome::Done(()));
    assert_eq!(written(&log), ["analysis", "later"]);
}

#[test]
fn what_is_kept_for_a_lane_is_left_alone_by_the_lane_above() {
    let mut budget = budget(4, [1; Lane::COUNT]);
    budget.lanes[Lane::Background.index()].reserve = 1;
    let scheduler = Scheduler::new(Config::new(budget));
    let gate = Gate::new();
    // Wide as it likes, the export is granted all but the core kept.
    let (export, cores) = hold_wide(&scheduler, Lane::Foreground, &gate, usize::MAX);
    assert_eq!(cores, 3);

    // So the small job below starts beside it, and is not asked to stand
    // aside when it checks.
    let small = scheduler.submit(Job::new(Lane::Background, |ctx| {
        ctx.checkpoint()?;
        Ok::<_, JobError<()>>(ctx.threads())
    }));
    assert_eq!(outcome(small), Outcome::Done(1));
    assert_eq!(state_of(&scheduler, &export), Some(JobState::Running));
    gate.open();
    assert_eq!(outcome(export), Outcome::Done(()));
}

#[test]
fn a_wide_job_at_the_front_is_not_overtaken_for_cores() {
    let scheduler = Scheduler::new(Config::new(budget(4, [1, 1, 1, 1, 3, 1])));
    let gate = Gate::new();
    let (first, cores) = hold_wide(&scheduler, Lane::Background, &gate, 3);
    assert_eq!(cores, 3);

    let log = log();
    let wide = scheduler.submit(named(Lane::Background, "wide", &log).at_least(4));
    let narrow = scheduler.submit(named(Lane::Background, "narrow", &log));
    // A core is free and so is a thread, and the narrow job could have
    // them - but then the wide one would never get its four.
    assert_eq!(scheduler.snapshot().cores, 3);
    assert_eq!(state_of(&scheduler, &wide), Some(JobState::Queued));
    assert_eq!(state_of(&scheduler, &narrow), Some(JobState::Queued));

    gate.open();
    assert_eq!(outcome(first), Outcome::Done(()));
    assert_eq!(outcome(wide), Outcome::Done(()));
    assert_eq!(outcome(narrow), Outcome::Done(()));
    assert_eq!(written(&log), ["wide", "narrow"]);
}

#[test]
fn a_job_that_asks_for_more_than_its_lane_can_ever_have_still_starts() {
    let scheduler = small(4);
    let job = scheduler.submit(
        Job::infallible(Lane::Background, |ctx| ctx.threads())
            .at_least(64)
            .wide(),
    );
    assert_eq!(outcome(job), Outcome::Done(4));
}

#[test]
fn a_lane_runs_no_more_jobs_at_once_than_it_has_threads() {
    let scheduler = Scheduler::new(Config::new(budget(4, [1, 1, 1, 1, 1, 2])));
    let gate = Gate::new();
    let first = hold(&scheduler, Lane::Io, &gate);
    let second = hold(&scheduler, Lane::Io, &gate);
    let third = scheduler.submit(Job::infallible(Lane::Io, |_| ()));
    assert_eq!(state_of(&scheduler, &third), Some(JobState::Queued));
    let load = scheduler.load()[Lane::Io.index()];
    assert_eq!((load.running, load.queued, load.cores), (2, 1, 0));

    gate.open();
    for job in [first, second, third] {
        assert_eq!(outcome(job), Outcome::Done(()));
    }
}

#[test]
fn a_full_lane_refuses_the_next_job_and_says_so() {
    let scheduler =
        Scheduler::new(Config::new(budget(1, [1; Lane::COUNT])).queue_limit(Lane::Background, 2));
    let gate = Gate::new();
    let holder = hold(&scheduler, Lane::Background, &gate);
    let log = log();
    let first = scheduler.submit(named(Lane::Background, "first", &log));
    let second = scheduler.submit(named(Lane::Background, "second", &log));

    let third = scheduler.submit(named(Lane::Background, "third", &log));
    // Over already, with nothing to wait for.
    assert!(third.is_finished());
    assert_eq!(outcome(third), Outcome::Refused(Refusal::QueueFull));
    // Another lane's line is another line.
    let elsewhere = scheduler.submit(named(Lane::Io, "elsewhere", &log));
    assert_eq!(outcome(elsewhere), Outcome::Done(()));

    gate.open();
    for job in [holder, first, second] {
        assert_eq!(outcome(job), Outcome::Done(()));
    }
    assert_eq!(written(&log), ["elsewhere", "first", "second"]);
}

#[test]
fn a_job_handed_in_from_inside_a_job_runs() {
    let scheduler = std::sync::Arc::new(small(2));
    let inner = std::sync::Arc::clone(&scheduler);
    let (handed, hands) = mpsc::channel();
    let outer = scheduler.submit(Job::infallible(Lane::Foreground, move |_| {
        let ticket = inner.submit_then(Job::infallible(Lane::Io, |_| 7), move |outcome| {
            handed.send(outcome).expect("the test is listening");
        });
        ticket.id()
    }));
    let Outcome::Done(id) = outcome(outer) else {
        panic!("the outer job did not finish");
    };
    assert_eq!(next(&hands), Outcome::Done(7));
    assert!(id.get() > 0);
}
