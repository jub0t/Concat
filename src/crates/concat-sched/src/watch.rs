// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! What the scheduler shows of itself: a picture of the moment, and each
//! job's turns as they happen.
//!
//! A job "stuck at 0%" used to be a guess between a job that was slow and
//! a job that had not started. The [`Snapshot`] says which, and what is in
//! front of it; the [`Event`]s are what a log line or a busy indicator is
//! made from.

use std::time::Duration;

use crate::budget::Lane;
use crate::job::{End, JobInfo, Resource};

/// Something that happened to a job.
///
/// Sent to the function given in
/// [`Config::events`](crate::Config::events), on the thread it happened
/// on - a worker's for a job that ran, the caller's for a job that ended
/// without running - and with no lock of the scheduler's held, so the
/// function may ask the scheduler anything. It should be quick all the
/// same: a job's thread is held while it runs.
///
/// One job's events come in order. A job that never ran has only its
/// `Finished`.
#[derive(Clone, PartialEq, Debug)]
pub enum Event {
    /// A thread has taken the job and is about to run it.
    Started(JobInfo),
    /// The job has stood aside at a checkpoint and given its cores back.
    Paused(JobInfo),
    /// The job has its cores again and runs on.
    Resumed(JobInfo),
    /// The job is over.
    Finished {
        /// The job as it ended.
        job: JobInfo,
        /// How it ended.
        end: End,
        /// How long it ran, or `None` if it never did.
        ran: Option<Duration>,
    },
}

impl Event {
    /// The job the event is about.
    pub fn job(&self) -> &JobInfo {
        match self {
            Event::Started(job)
            | Event::Paused(job)
            | Event::Resumed(job)
            | Event::Finished { job, .. } => job,
        }
    }
}

/// One lane at one moment.
#[derive(Clone, PartialEq, Debug)]
pub struct LaneSnapshot {
    /// Which lane.
    pub lane: Lane,
    /// The lane's threads.
    pub workers: usize,
    /// The cores its running jobs hold; nothing for a lane outside the
    /// pool.
    pub cores: usize,
    /// The jobs a thread has, oldest first, the paused among them.
    pub running: Vec<JobInfo>,
    /// The jobs waiting, in the order they will be looked at.
    pub queued: Vec<JobInfo>,
}

/// How much of a resource there is and how much is held.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ResourceUse {
    /// Which resource.
    pub resource: Resource,
    /// How much there is in all.
    pub capacity: u64,
    /// How much the running jobs hold.
    pub held: u64,
}

/// Everything waiting and running, at one moment.
///
/// Made under the scheduler's one lock in a single pass over the jobs it
/// has, which are bounded; it is meant to be asked for every frame.
#[derive(Clone, PartialEq, Debug)]
pub struct Snapshot {
    /// Each lane, in [`Lane::ALL`] order.
    pub lanes: [LaneSnapshot; Lane::COUNT],
    /// The cores the sharing lanes divide.
    pub pool: usize,
    /// The cores held across those lanes. More than the pool while a job
    /// asked to stand aside has not yet reached a checkpoint.
    pub cores: usize,
    /// The resources the host has declared.
    pub resources: Vec<ResourceUse>,
    /// How many inputs are remembered as having failed.
    pub failures: usize,
}

impl Snapshot {
    /// One lane's part.
    pub fn lane(&self, lane: Lane) -> &LaneSnapshot {
        &self.lanes[lane.index()]
    }

    /// Every job, the running before the waiting, lane by lane.
    pub fn jobs(&self) -> impl Iterator<Item = &JobInfo> {
        self.lanes
            .iter()
            .flat_map(|lane| lane.running.iter().chain(lane.queued.iter()))
    }

    /// The job with this number, if it is still waiting or running.
    pub fn job(&self, id: crate::JobId) -> Option<&JobInfo> {
        self.jobs().find(|job| job.id == id)
    }

    /// Whether nothing is waiting and nothing is running.
    pub fn is_idle(&self) -> bool {
        self.jobs().next().is_none()
    }
}

/// How busy one lane is, without the jobs.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct LaneLoad {
    /// Jobs a thread has.
    pub running: usize,
    /// Jobs waiting.
    pub queued: usize,
    /// Cores held.
    pub cores: usize,
}
