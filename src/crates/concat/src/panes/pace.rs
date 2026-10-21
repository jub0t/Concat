// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! How far along a transcription or a read looks, from what it reports.
//!
//! The jobs report rarely: whisper once per thirty seconds of sound, Kokoro
//! and Pocket once per sentence, and nothing at all while the sound is read
//! in or the model loads. A bar fed only those numbers sits still for most
//! of a run and then jumps. [`Pace`] lays the run out as its stages, each
//! given room on the bar by how long it is expected to take, and for each
//! report says three things: where the bar surely is (`floor`), where the
//! step under way will leave it (`ceiling`), and how long that step should
//! take (`step`). The bar moves from floor toward ceiling in that time and
//! never reaches the ceiling before the job says so; see `Progress` in
//! primitives/modal.slint.
//!
//! The expectations start as estimates from the model's size and the
//! input's length, and are replaced by what the run measures once the work
//! has reported once: from then on the step and the time left are timed,
//! not guessed. [`Learned`] carries a finished run's timings to the next
//! run on the same model.

use std::collections::HashMap;
use std::time::Instant;

use concat_speech::JobStage;

/// A stage of a run, as the plan names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Reading,
    Loading,
    Working,
}

/// One run's progress, as the bar draws it.
#[derive(Clone, Debug)]
pub struct Pace {
    /// The stages the run will report, in order, each with the seconds it
    /// is expected to take.
    plan: Vec<(Stage, f32)>,
    /// The stage under way, by its row in `plan`.
    at: Option<usize>,
    /// When the stage under way began.
    since: Instant,
    /// When the work began, which is what its rate is timed from.
    working: Option<Instant>,
    /// How long the model took to load, when this run loaded it.
    loaded_in: Option<f32>,
    /// Where the bar surely is, `0..=1`.
    pub floor: f32,
    /// Where the step under way will leave it, `floor..=1`.
    pub ceiling: f32,
    /// Seconds the step under way is expected to take.
    pub step: f32,
    /// Seconds of the run still to go, once the work has been timed;
    /// negative before.
    pub left: f32,
}

impl Default for Pace {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl Pace {
    /// A run that will go through `plan`'s stages, in order. A stage the
    /// run turns out to skip (a model already loaded) is passed over when
    /// the next one is reported.
    pub fn new(plan: Vec<(Stage, f32)>) -> Self {
        let mut pace = Self {
            plan,
            at: None,
            since: Instant::now(),
            working: None,
            loaded_in: None,
            floor: 0.0,
            ceiling: 0.0,
            step: 0.0,
            left: -1.0,
        };
        // Before the first report the run heads for the end of its first
        // stage that takes any time.
        if let Some(index) = pace.first() {
            pace.ceiling = pace.span(index).1;
            pace.step = pace.plan[index].1;
        }
        pace
    }

    /// The stage the run is in: the one it last reported, or before any
    /// report the first that takes any time.
    pub fn stage(&self) -> Option<Stage> {
        self.at
            .or_else(|| self.first())
            .map(|index| self.plan[index].0)
    }

    /// The first stage in the plan expected to take any time.
    fn first(&self) -> Option<usize> {
        self.plan.iter().position(|&(_, seconds)| seconds > 0.0)
    }

    /// Where on the bar stage `index` runs, from and to.
    fn span(&self, index: usize) -> (f32, f32) {
        let total: f32 = self.plan.iter().map(|(_, seconds)| seconds).sum();
        if total <= 0.0 {
            return (0.0, 1.0);
        }
        let before: f32 = self.plan[..index].iter().map(|(_, seconds)| seconds).sum();
        let start = before / total;
        (start, start + self.plan[index].1 / total)
    }

    /// Takes in one report from the run.
    pub fn report(&mut self, stage: JobStage) {
        let named = match stage {
            JobStage::Reading => Stage::Reading,
            JobStage::Loading => Stage::Loading,
            JobStage::Working { .. } => Stage::Working,
        };
        let Some(index) = self.plan.iter().position(|(planned, _)| *planned == named) else {
            return;
        };
        if self.at != Some(index) {
            if self.at.is_some_and(|at| self.plan[at].0 == Stage::Loading) {
                self.loaded_in = Some(self.since.elapsed().as_secs_f32());
            }
            self.at = Some(index);
            self.since = Instant::now();
        }
        let (start, end) = self.span(index);
        let (done, next) = match stage {
            JobStage::Working { done, next } => {
                let done = done.clamp(0.0, 1.0);
                (done, next.clamp(done, 1.0))
            }
            _ => (0.0, 1.0),
        };
        self.floor = self.floor.max(start + done * (end - start));
        self.ceiling = (start + next * (end - start)).max(self.floor);
        let planned = self.plan[index].1;
        self.step = planned * (next - done);
        self.left = -1.0;
        if named == Stage::Working {
            let working = *self.working.get_or_insert_with(Instant::now);
            // Timed once a little of the work is done: before that the
            // rate is mostly the time to the first report.
            if done >= 0.02 {
                let per = working.elapsed().as_secs_f32() / done;
                self.step = per * (next - done);
                self.left = per * (1.0 - done);
            }
        }
    }

    /// What this run measured, for the next one on the same model: the
    /// load, when it loaded, and the work's seconds per `units` of input.
    pub fn learned(&self, units: f32) -> Learned {
        Learned {
            load: self.loaded_in,
            per_unit: self
                .working
                .filter(|_| units > 0.0)
                .map(|working| working.elapsed().as_secs_f32() / units),
        }
    }
}

/// A finished run's timings, kept for the next run on the same model.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Learned {
    /// Seconds the model took to load.
    pub load: Option<f32>,
    /// Seconds of work per unit of input: per thirty-second window for a
    /// transcription, per character for a read.
    pub per_unit: Option<f32>,
}

/// The timings learned this session, by model id.
#[derive(Clone, Debug, Default)]
pub struct Timings(HashMap<String, Learned>);

impl Timings {
    pub fn get(&self, model: &str) -> Learned {
        self.0.get(model).copied().unwrap_or_default()
    }

    /// Keeps what a finished run measured, over what an earlier one did; a
    /// run that did not load the model keeps the earlier load.
    pub fn keep(&mut self, model: &str, learned: Learned) {
        let kept = self.0.entry(model.to_owned()).or_default();
        kept.load = learned.load.or(kept.load);
        kept.per_unit = learned.per_unit.or(kept.per_unit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    /// Each stage has room on the bar by its expected time, and a stage's
    /// report heads the bar for the end of it.
    #[test]
    fn stages_take_room_by_their_expected_time() {
        let mut pace = Pace::new(vec![
            (Stage::Reading, 1.0),
            (Stage::Loading, 3.0),
            (Stage::Working, 6.0),
        ]);
        assert!(close(pace.ceiling, 0.1));
        pace.report(JobStage::Reading);
        assert!(close(pace.floor, 0.0) && close(pace.ceiling, 0.1));
        assert!(close(pace.step, 1.0));
        pace.report(JobStage::Loading);
        assert!(close(pace.floor, 0.1) && close(pace.ceiling, 0.4));
        assert!(close(pace.step, 3.0));
        pace.report(JobStage::Working {
            done: 0.0,
            next: 0.5,
        });
        assert!(close(pace.floor, 0.4) && close(pace.ceiling, 0.7));
        assert!(close(pace.step, 3.0));
        assert!(pace.left < 0.0, "nothing timed yet");
        pace.report(JobStage::Working {
            done: 0.5,
            next: 1.0,
        });
        assert!(close(pace.floor, 0.7) && close(pace.ceiling, 1.0));
        assert!(pace.left >= 0.0, "timed once the work has reported");
    }

    /// A stage the run skips - the model was still loaded - is passed
    /// over, and the bar never goes back.
    #[test]
    fn a_skipped_stage_is_passed_over_and_the_bar_never_goes_back() {
        let mut pace = Pace::new(vec![
            (Stage::Reading, 1.0),
            (Stage::Loading, 1.0),
            (Stage::Working, 2.0),
        ]);
        pace.report(JobStage::Reading);
        pace.report(JobStage::Working {
            done: 0.0,
            next: 1.0,
        });
        assert!(close(pace.floor, 0.5) && close(pace.ceiling, 1.0));
        pace.report(JobStage::Reading);
        assert!(close(pace.floor, 0.5), "a late report cannot move it back");
        assert!(pace.ceiling >= pace.floor);
    }

    /// A model still loaded takes no time: before any report the run is
    /// in, and heads for the end of, the first stage that does.
    #[test]
    fn a_stage_of_no_time_is_not_the_one_shown_first() {
        let pace = Pace::new(vec![(Stage::Loading, 0.0), (Stage::Working, 4.0)]);
        assert_eq!(pace.stage(), Some(Stage::Working));
        assert!(close(pace.ceiling, 1.0) && close(pace.step, 4.0));
    }

    /// Reports outside the plan, or with numbers out of range, change
    /// nothing they should not.
    #[test]
    fn reports_out_of_range_are_held_in() {
        let mut pace = Pace::new(vec![(Stage::Working, 1.0)]);
        pace.report(JobStage::Loading);
        assert!(close(pace.floor, 0.0));
        pace.report(JobStage::Working {
            done: 1.5,
            next: 0.2,
        });
        assert!(close(pace.floor, 1.0) && close(pace.ceiling, 1.0));
        let empty = Pace::default();
        assert!(close(empty.floor, 0.0) && close(empty.ceiling, 0.0));
    }

    /// A run that did not load keeps the earlier run's load.
    #[test]
    fn timings_keep_the_last_measure_of_each() {
        let mut timings = Timings::default();
        timings.keep(
            "base",
            Learned {
                load: Some(2.0),
                per_unit: Some(1.0),
            },
        );
        timings.keep(
            "base",
            Learned {
                load: None,
                per_unit: Some(0.5),
            },
        );
        assert_eq!(
            timings.get("base"),
            Learned {
                load: Some(2.0),
                per_unit: Some(0.5),
            }
        );
        assert_eq!(timings.get("other"), Learned::default());
    }
}
