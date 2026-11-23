// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The lanes, and how much of the machine each one gets.
//!
//! Every part of the editor used to pick its own share of the machine and
//! nothing added them up: four threads for the window's short jobs, three
//! to six for the long ones, two to four decoders, a model's eight. On two
//! cores that was dozens of threads wanting to run at once, and on
//! thirty-two most of those caps stopped at four, six or eight and the
//! rest of the machine sat idle. [`Budget::for_machine`] is the one place
//! that does the sum. It is a pure function of the [`Machine`], so the
//! whole table can be read, and is tested, without the machine in question.

use std::fmt;

/// A lane of work, most urgent first.
///
/// Sound is not here. It is mixed on a real-time thread of its own that
/// the output device calls, and nothing may be queued in front of it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Lane {
    /// The frame on screen: the picture under the playhead and its refine.
    /// Its own threads, which nothing else is ever run on.
    Frame,
    /// A project's save. One thread of its own, so a save waits behind
    /// nothing but another save, and no two ever write at once.
    Save,
    /// What the person asked for and is waiting on: an export, a merge.
    Foreground,
    /// Decoding ahead of the playhead, and the small pictures the window
    /// is waiting to draw: filmstrips, cards, waveforms.
    Prefetch,
    /// Analysis and models nobody is watching: cutouts, stabilization,
    /// captions, beats, proxies.
    Background,
    /// Reading and writing files and the network: a probe, a download, a
    /// copy. Its threads spend their time blocked, not computing.
    Io,
}

impl Lane {
    /// How many lanes there are.
    pub const COUNT: usize = 6;

    /// Every lane, most urgent first.
    pub const ALL: [Lane; Lane::COUNT] = [
        Lane::Frame,
        Lane::Save,
        Lane::Foreground,
        Lane::Prefetch,
        Lane::Background,
        Lane::Io,
    ];

    /// The lanes that share the machine's cores between them, most urgent
    /// first; see [`Lane::shares_cores`].
    pub const SHARING: [Lane; 3] = [Lane::Foreground, Lane::Prefetch, Lane::Background];

    /// The lane's place in [`Lane::ALL`].
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The lane's name, as its threads and the log say it.
    pub const fn name(self) -> &'static str {
        match self {
            Lane::Frame => "frame",
            Lane::Save => "save",
            Lane::Foreground => "foreground",
            Lane::Prefetch => "prefetch",
            Lane::Background => "background",
            Lane::Io => "io",
        }
    }

    /// Whether the lane's jobs are counted against the shared pool of
    /// cores.
    ///
    /// The three lanes of long, computing work are. The frame and the save
    /// are not: each must start the moment it is asked for, and each is
    /// over quickly. Nor is file and network work, which waits on a disk
    /// or a socket and leaves its core to someone else while it does.
    pub const fn shares_cores(self) -> bool {
        matches!(self, Lane::Foreground | Lane::Prefetch | Lane::Background)
    }
}

impl fmt::Display for Lane {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The machine, as far as sizing the lanes goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Machine {
    /// Hardware threads the process may run on.
    pub cores: usize,
    /// Physical memory in bytes, or `None` where the platform will not say.
    pub ram_bytes: Option<u64>,
}

impl Machine {
    /// A machine with `cores` hardware threads and `ram_bytes` of memory.
    pub const fn new(cores: usize, ram_bytes: u64) -> Machine {
        Machine {
            cores,
            ram_bytes: Some(ram_bytes),
        }
    }

    /// This machine: its hardware threads as the system reports them, and
    /// the memory the caller has read.
    ///
    /// The memory is the caller's to find because finding it takes a
    /// system call per platform, and this crate has no unsafe code;
    /// `concat_host::memory::total` already asks.
    pub fn detect(ram_bytes: Option<u64>) -> Machine {
        Machine {
            cores: std::thread::available_parallelism().map_or(2, |count| count.get()),
            ram_bytes,
        }
    }

    /// Memory in whole gibibytes. A machine that will not say is taken to
    /// have two per core, between four and sixty-four: what machines of
    /// its size are sold with.
    fn gibibytes(self) -> u64 {
        const GIBIBYTE: u64 = 1 << 30;
        match self.ram_bytes {
            Some(bytes) => (bytes / GIBIBYTE).max(1),
            None => (self.cores as u64 * 2).clamp(4, 64),
        }
    }
}

/// One lane's share of the machine.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct LaneBudget {
    /// The lane's threads, and so the most jobs it runs at once.
    pub workers: usize,
    /// Cores of the pool kept for this lane: the lanes above it never take
    /// them, so its small jobs keep moving under an export. Nothing for a
    /// lane outside the pool.
    pub reserve: usize,
    /// The most cores one job in the lane is granted for its own threads.
    pub width: usize,
}

/// How the machine is divided between the lanes.
///
/// Two currencies. A lane's `workers` are threads: how many of its jobs
/// run at once. The `pool` is cores: every job on a lane that
/// [shares the cores](Lane::shares_cores) is granted some for the threads
/// it starts itself - an encoder's, a model's - and the grants within a
/// lane and the lanes above it never add up to more than the pool. All
/// the lanes together can, for a while: by a job below that has been
/// asked to stand aside for a lane above and has not yet reached its
/// next checkpoint, or by less than half of one that was not asked
/// because so little of it was wanted. The scheduler's snapshot says
/// when.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Budget {
    /// The hardware threads the budget was made for.
    pub cores: usize,
    /// The cores the sharing lanes divide between them.
    pub pool: usize,
    /// Each lane's share, in [`Lane::ALL`] order.
    pub lanes: [LaneBudget; Lane::COUNT],
}

impl Budget {
    /// The budget for `machine`.
    ///
    /// The rules, and why each is what it is:
    ///
    /// - **The pool is every core but one.** The one is the window's own
    ///   thread, the mixer and the frame on screen, which are not counted
    ///   and must not find the machine full.
    /// - **The frame has two threads**, on every machine: the frame asked
    ///   for and the refine that walks to the exact picture. The monitor
    ///   lets a scrub's next frame start while a refine is still out - the
    ///   frame a refine walks to can be most of a second away - so on one
    ///   thread the hand would wait behind it, and it is the weak machine
    ///   whose refines are slow. It never has more in flight whatever the
    ///   machine, so the lane does not grow with it.
    /// - **The save has one thread**, always.
    /// - **A foreground job may have the whole pool** less what is kept
    ///   for the lanes below. An export fans out over the cores itself, so
    ///   two side by side buy progress on each and no speed; the lane's
    ///   further threads are for jobs that ask for less. A job there holds
    ///   frames in flight, so memory caps the threads at one per 8 GiB.
    /// - **Prefetch has a thread per two cores**, two at least and eight
    ///   at most, and each job one core: a decoder walks from a keyframe
    ///   on one thread. From eight cores one core is kept for it, two from
    ///   thirty-two, so the picture still plays under an export.
    /// - **Background has a thread per two cores**, six at most, and one
    ///   per 4 GiB: each job there holds a model. A job may have the whole
    ///   pool, so an analysis alone on an idle machine uses all of it.
    ///   From sixteen cores one core is kept for it.
    /// - **File and network work has a thread per core**, two at least and
    ///   eight at most. They wait on the disk and the network, so they
    ///   are not counted against the cores.
    pub fn for_machine(machine: Machine) -> Budget {
        let cores = machine.cores.max(1);
        let gibibytes = usize::try_from(machine.gibibytes()).unwrap_or(usize::MAX);
        let pool = cores.saturating_sub(1).max(1);

        let prefetch_reserve = match cores {
            0..=7 => 0,
            8..=31 => 1,
            _ => 2,
        };
        let background_reserve = usize::from(cores >= 16);

        let mut lanes = [LaneBudget::default(); Lane::COUNT];
        lanes[Lane::Frame.index()] = LaneBudget {
            workers: 2,
            reserve: 0,
            width: 1,
        };
        lanes[Lane::Save.index()] = LaneBudget {
            workers: 1,
            reserve: 0,
            width: 1,
        };
        lanes[Lane::Foreground.index()] = LaneBudget {
            workers: (cores / 4).clamp(1, 4).min((gibibytes / 8).max(1)),
            reserve: 0,
            width: pool,
        };
        lanes[Lane::Prefetch.index()] = LaneBudget {
            workers: (cores / 2).clamp(2, 8),
            reserve: prefetch_reserve,
            width: 1,
        };
        lanes[Lane::Background.index()] = LaneBudget {
            workers: (cores / 2).clamp(1, 6).min((gibibytes / 4).max(1)),
            reserve: background_reserve,
            width: pool,
        };
        lanes[Lane::Io.index()] = LaneBudget {
            workers: cores.clamp(2, 8),
            reserve: 0,
            width: 1,
        };
        Budget { cores, pool, lanes }.checked()
    }

    /// One lane's share.
    pub fn lane(&self, lane: Lane) -> &LaneBudget {
        &self.lanes[lane.index()]
    }

    /// The most cores `lane` can ever hold: the pool, less what is kept
    /// for the lanes below it. Nothing for a lane outside the pool.
    pub fn entitlement(&self, lane: Lane) -> usize {
        if !lane.shares_cores() {
            return 0;
        }
        let below: usize = Lane::SHARING
            .iter()
            .filter(|other| **other > lane)
            .map(|other| self.lane(*other).reserve)
            .sum();
        self.pool.saturating_sub(below)
    }

    /// Every thread the scheduler starts.
    pub fn threads(&self) -> usize {
        self.lanes.iter().map(|lane| lane.workers).sum()
    }

    /// The most threads that can be computing at once on the scheduler's
    /// account: the pool, and the frame's own threads on top. The save and
    /// the file and network threads are left out; they wait on a disk.
    pub fn peak(&self) -> usize {
        self.pool + self.lane(Lane::Frame).workers
    }

    /// This budget with every figure brought within the rules, for one
    /// written by hand: every lane has a thread, the save exactly one, the
    /// pool a core, the lanes outside the pool keep nothing, and what is
    /// kept for the lanes inside it leaves a core for each lane above.
    pub fn checked(mut self) -> Budget {
        self.cores = self.cores.max(1);
        self.pool = self.pool.max(1);
        for lane in Lane::ALL {
            let share = &mut self.lanes[lane.index()];
            share.workers = share.workers.max(1);
            share.width = share.width.max(1);
            if !lane.shares_cores() {
                share.reserve = 0;
            }
        }
        self.lanes[Lane::Save.index()].workers = 1;
        // Nothing is above the first sharing lane to keep cores from.
        self.lanes[Lane::SHARING[0].index()].reserve = 0;
        // What is kept comes to no more than the pool less one, so that
        // every lane still has a core it can reach. The more urgent lane
        // keeps its figure first; the lanes below it are trimmed.
        let mut spare = self.pool - 1;
        for lane in Lane::SHARING {
            let share = &mut self.lanes[lane.index()];
            share.reserve = share.reserve.min(spare);
            spare -= share.reserve;
        }
        for lane in Lane::SHARING {
            let most = self.entitlement(lane);
            let share = &mut self.lanes[lane.index()];
            share.width = share.width.min(most);
        }
        self
    }
}

impl fmt::Display for Budget {
    /// One line for the log: the cores, the pool, and each lane's threads
    /// with the cores a job gets and the cores kept, where there are any.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} cores, a pool of {}", self.cores, self.pool)?;
        for lane in Lane::ALL {
            let share = self.lane(lane);
            write!(f, "; {} {}", lane.name(), share.workers)?;
            if lane.shares_cores() {
                write!(f, " (up to {} wide", share.width)?;
                if share.reserve > 0 {
                    write!(f, ", {} kept", share.reserve)?;
                }
                f.write_str(")")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIBIBYTE: u64 = 1 << 30;

    fn budget(cores: usize, gibibytes: u64) -> Budget {
        Budget::for_machine(Machine::new(cores, gibibytes * GIBIBYTE))
    }

    /// `(workers, reserve, width)` of each lane, in [`Lane::ALL`] order.
    fn rows(budget: &Budget) -> [(usize, usize, usize); Lane::COUNT] {
        budget
            .lanes
            .map(|lane| (lane.workers, lane.reserve, lane.width))
    }

    #[test]
    fn the_table_of_five_machines_is_what_the_rules_say() {
        // frame, save, foreground, prefetch, background, io
        let two = budget(2, 4);
        assert_eq!(two.pool, 1);
        assert_eq!(
            rows(&two),
            [
                (2, 0, 1),
                (1, 0, 1),
                (1, 0, 1),
                (2, 0, 1),
                (1, 0, 1),
                (2, 0, 1)
            ]
        );

        let four = budget(4, 8);
        assert_eq!(four.pool, 3);
        assert_eq!(
            rows(&four),
            [
                (2, 0, 1),
                (1, 0, 1),
                (1, 0, 3),
                (2, 0, 1),
                (2, 0, 3),
                (4, 0, 1)
            ]
        );

        let eight = budget(8, 16);
        assert_eq!(eight.pool, 7);
        assert_eq!(
            rows(&eight),
            [
                (2, 0, 1),
                (1, 0, 1),
                (2, 0, 6),
                (4, 1, 1),
                (4, 0, 7),
                (8, 0, 1)
            ]
        );

        let sixteen = budget(16, 64);
        assert_eq!(sixteen.pool, 15);
        assert_eq!(
            rows(&sixteen),
            [
                (2, 0, 1),
                (1, 0, 1),
                (4, 0, 13),
                (8, 1, 1),
                (6, 1, 15),
                (8, 0, 1)
            ]
        );

        let thirty_two = budget(32, 128);
        assert_eq!(thirty_two.pool, 31);
        assert_eq!(
            rows(&thirty_two),
            [
                (2, 0, 1),
                (1, 0, 1),
                (4, 0, 28),
                (8, 2, 1),
                (6, 1, 31),
                (8, 0, 1)
            ]
        );
    }

    #[test]
    fn a_weak_machine_is_never_given_more_to_compute_than_it_has_cores() {
        // The refine is a second frame thread, and the one thread over,
        // while a refine and a frame are both out, is the price of a scrub
        // that never waits behind one.
        for (cores, gibibytes) in [(2, 4), (3, 4), (4, 8), (6, 8)] {
            let budget = budget(cores, gibibytes);
            assert!(
                budget.peak() <= cores + 1,
                "{cores} cores: the pool and the frame come to {}",
                budget.peak()
            );
        }
        for cores in [8, 12, 16, 32, 64, 128] {
            let budget = budget(cores, 64);
            assert!(budget.peak() <= cores + 1, "{cores} cores");
        }
    }

    #[test]
    fn a_strong_machine_is_not_left_idle() {
        for (cores, gibibytes) in [(4, 8), (8, 16), (16, 64), (32, 128), (64, 256)] {
            let budget = budget(cores, gibibytes);
            // Everything but the one core kept for the window is there to
            // be used...
            assert_eq!(budget.pool, cores - 1, "{cores} cores");
            // ...an export alone gets at least three quarters of the
            // machine...
            let export = budget.lane(Lane::Foreground).width;
            assert!(
                export * 4 >= cores * 3,
                "{cores} cores: an export gets {export}"
            );
            // ...and an analysis alone, with nothing above it running,
            // gets the whole pool.
            assert_eq!(budget.lane(Lane::Background).width, budget.pool);
            assert_eq!(budget.entitlement(Lane::Background), budget.pool);
        }
        // What the old caps stopped at whatever the machine.
        let big = budget(32, 128);
        assert!(big.lane(Lane::Foreground).width > 8);
        assert!(big.lane(Lane::Prefetch).workers > 4);
    }

    #[test]
    fn memory_and_not_cores_caps_the_jobs_that_hold_a_model() {
        let starved = budget(16, 8);
        assert_eq!(starved.lane(Lane::Background).workers, 2);
        assert_eq!(starved.lane(Lane::Foreground).workers, 1);
        let fed = budget(16, 64);
        assert_eq!(fed.lane(Lane::Background).workers, 6);
        assert_eq!(fed.lane(Lane::Foreground).workers, 4);
        // The cores a job is granted are the machine's all the same.
        assert_eq!(starved.pool, fed.pool);
    }

    #[test]
    fn a_machine_that_will_not_say_its_memory_is_sized_by_its_cores() {
        let unknown = Budget::for_machine(Machine {
            cores: 8,
            ram_bytes: None,
        });
        assert_eq!(unknown, budget(8, 16));
    }

    #[test]
    fn every_lane_of_every_machine_has_a_thread_and_a_core_within_reach() {
        for cores in 0..=160 {
            for gibibytes in [1, 2, 4, 8, 16, 32, 64, 128, 512] {
                let budget = budget(cores, gibibytes);
                assert!(budget.pool >= 1);
                assert!(budget.pool <= cores.max(1));
                assert_eq!(budget.lane(Lane::Save).workers, 1);
                for lane in Lane::ALL {
                    let share = budget.lane(lane);
                    assert!(share.workers >= 1, "{cores} cores, {lane}");
                    assert!(share.width >= 1, "{cores} cores, {lane}");
                    if lane.shares_cores() {
                        let most = budget.entitlement(lane);
                        assert!(most >= 1, "{cores} cores, {lane}");
                        assert!(share.width <= most, "{cores} cores, {lane}");
                    } else {
                        assert_eq!(share.reserve, 0);
                    }
                }
                assert_eq!(budget, budget.checked(), "{cores} cores");
            }
        }
    }

    #[test]
    fn more_cores_never_means_less_of_anything() {
        let mut before = budget(1, 512);
        for cores in 2..=160 {
            let after = budget(cores, 512);
            assert!(after.pool >= before.pool);
            for lane in Lane::ALL {
                assert!(
                    after.lane(lane).workers >= before.lane(lane).workers,
                    "{cores} cores, {lane}"
                );
                assert!(
                    after.lane(lane).width >= before.lane(lane).width,
                    "{cores} cores, {lane}"
                );
            }
            before = after;
        }
    }

    #[test]
    fn a_budget_written_by_hand_is_brought_within_the_rules() {
        let wild = Budget {
            cores: 0,
            pool: 0,
            lanes: [LaneBudget {
                workers: 0,
                reserve: 9,
                width: 0,
            }; Lane::COUNT],
        }
        .checked();
        assert_eq!(wild.cores, 1);
        assert_eq!(wild.pool, 1);
        for lane in Lane::ALL {
            assert_eq!(wild.lane(lane).workers, 1);
            assert_eq!(wild.lane(lane).reserve, 0);
            assert_eq!(wild.lane(lane).width, 1);
        }

        let mut greedy = budget(8, 16);
        greedy.lanes[Lane::Save.index()].workers = 3;
        greedy.lanes[Lane::Frame.index()].reserve = 2;
        greedy.lanes[Lane::Prefetch.index()].reserve = 5;
        greedy.lanes[Lane::Background.index()].reserve = 5;
        let greedy = greedy.checked();
        assert_eq!(greedy.lane(Lane::Save).workers, 1);
        assert_eq!(greedy.lane(Lane::Frame).reserve, 0);
        // Six of the seven may be kept; the more urgent lane keeps its own.
        assert_eq!(greedy.lane(Lane::Prefetch).reserve, 5);
        assert_eq!(greedy.lane(Lane::Background).reserve, 1);
        assert_eq!(greedy.entitlement(Lane::Foreground), 1);
        assert_eq!(greedy.lane(Lane::Foreground).width, 1);
    }

    #[test]
    fn the_budget_reads_as_one_line() {
        assert_eq!(
            budget(16, 64).to_string(),
            "16 cores, a pool of 15; frame 2; save 1; foreground 4 (up to 13 wide); \
             prefetch 8 (up to 1 wide, 1 kept); background 6 (up to 15 wide, 1 kept); io 8"
        );
    }
}
