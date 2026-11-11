// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A piece of work, timed step by step, and how it went.
//!
//! "Captions took forever on my laptop" cannot be answered from a log that
//! says only that they started and that they ended. An [`Op`] is the work
//! as one thing: it is opened, each step in it is named as it begins, and
//! when it ends the log gets one line that says the whole of it:
//!
//! ```text
//! INFO  captions: done in 43.2 s · 120 captions · small model, 2 m 14 s of sound · load model 1.21 s · find speech 812 ms · transcribe 40.9 s · split 96 ms
//! ERROR captions: failed after 3.10 s in transcribe: the model file is truncated · load model 1.21 s · transcribe 1.89 s
//! ```
//!
//! That is the line to compare across machines and across builds, and the
//! one to ask a reporter for. Each step is also written as it ends, at
//! debug, for following a run as it goes.
//!
//! ```
//! let mut op = concat_log::op!("captions");
//! op.note(format_args!("{} model", "small"));
//! op.step("load model");
//! // ...
//! op.step("transcribe");
//! // ...
//! op.done(format_args!("{} captions", 120));
//! ```
//!
//! An `Op` that is dropped without being told how it went - an early
//! return, a `?` - says so as a warning, with where it had got to, so no
//! piece of work disappears from the log half-way through.

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use crate::clock::span;

/// A piece of work being timed; see the module.
#[must_use = "an Op says how the work went when it is finished: call done, failed or cancelled"]
pub struct Op {
    target: &'static str,
    name: String,
    started: Instant,
    /// What is known about the work: the model, how much sound.
    notes: Vec<String>,
    /// The steps that have ended, and how long each took.
    steps: Vec<(String, Duration)>,
    /// The step under way, and when it began.
    current: Option<(String, Instant)>,
    finished: bool,
}

/// Opens an [`Op`] named for the work, in the module that does it:
/// `concat_log::op!("captions")`.
#[macro_export]
macro_rules! op {
    ($name:expr) => {
        $crate::Op::start(::core::module_path!(), $name)
    };
}

impl Op {
    /// Opens the work. `target` is the module it is logged from; the
    /// [`op!`](crate::op) macro fills it in.
    pub fn start(target: &'static str, name: impl Into<String>) -> Self {
        let name = name.into();
        log::debug!(target: target, "{name}: started");
        Op {
            target,
            name,
            started: Instant::now(),
            notes: Vec::new(),
            steps: Vec::new(),
            current: None,
            finished: false,
        }
    }

    /// Adds something known about the work to its line: which model, how
    /// much sound, how many frames. Said once, when it ends.
    pub fn note(&mut self, what: impl std::fmt::Display) {
        self.notes.push(what.to_string());
    }

    /// Begins a step, ending the one before it.
    pub fn step(&mut self, name: impl Into<String>) {
        self.end_step();
        self.current = Some((name.into(), Instant::now()));
    }

    /// Ends the step under way without beginning another: what follows is
    /// the work's own time, not that step's.
    pub fn end_step(&mut self) {
        if let Some((step, began)) = self.current.take() {
            let took = began.elapsed();
            log::debug!(target: self.target, "{}: {step} took {}", self.name, span(took));
            // The same step twice - a loop over clips - is one step that
            // took the sum.
            match self.steps.iter_mut().find(|(name, _)| *name == step) {
                Some((_, total)) => *total += took,
                None => self.steps.push((step, took)),
            }
        }
    }

    /// How long the work has been going.
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// The work ended well. `result` is what came of it - "120 captions" -
    /// or `""` when there is nothing to add.
    pub fn done(mut self, result: impl std::fmt::Display) {
        self.end_step();
        let mut line = format!("{}: done in {}", self.name, span(self.elapsed()));
        // What came of it first: a reader who stops there has the fact.
        let result = result.to_string();
        if !result.is_empty() {
            let _ = write!(line, " · {result}");
        }
        self.detail(&mut line);
        log::info!(target: self.target, "{line}");
        self.finished = true;
    }

    /// The work failed, and why.
    pub fn failed(mut self, error: impl std::fmt::Display) {
        let at = self.current.as_ref().map(|(step, _)| step.clone());
        self.end_step();
        let mut line = format!("{}: failed after {}", self.name, span(self.elapsed()));
        if let Some(step) = at {
            let _ = write!(line, " in {step}");
        }
        let _ = write!(line, ": {error}");
        self.detail(&mut line);
        log::error!(target: self.target, "{line}");
        self.finished = true;
    }

    /// The work was stopped on purpose - the person pressed Cancel. Not a
    /// failure, and not something to leave unsaid.
    pub fn cancelled(mut self) {
        let at = self.current.as_ref().map(|(step, _)| step.clone());
        self.end_step();
        let mut line = format!("{}: cancelled after {}", self.name, span(self.elapsed()));
        if let Some(step) = at {
            let _ = write!(line, " in {step}");
        }
        self.detail(&mut line);
        log::info!(target: self.target, "{line}");
        self.finished = true;
    }

    /// Ends the work as its result says: [`Op::done`] with nothing to add
    /// for an `Ok`, [`Op::failed`] with the error for an `Err`. The result
    /// is handed back, so this goes on the last line of a function.
    pub fn end<T, E: std::fmt::Display>(self, result: Result<T, E>) -> Result<T, E> {
        match &result {
            Ok(_) => self.done(""),
            Err(error) => self.failed(error),
        }
        result
    }

    /// The notes and the steps, onto the end of the work's line.
    fn detail(&self, line: &mut String) {
        if !self.notes.is_empty() {
            let _ = write!(line, " · {}", self.notes.join(", "));
        }
        for (step, took) in &self.steps {
            let _ = write!(line, " · {step} {}", span(*took));
        }
    }
}

impl Drop for Op {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let at = self.current.as_ref().map(|(step, _)| step.clone());
        self.end_step();
        let mut line = format!(
            "{}: left unfinished after {}",
            self.name,
            span(self.elapsed())
        );
        if let Some(step) = at {
            let _ = write!(line, " in {step}");
        }
        self.detail(&mut line);
        log::warn!(target: self.target, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An op with its clock wound back, so its line can be read exactly.
    fn op(steps: &[(&str, u64)]) -> Op {
        let mut op = Op::start("concat_speech::transcribe", "captions");
        op.steps = steps
            .iter()
            .map(|(name, millis)| ((*name).to_owned(), Duration::from_millis(*millis)))
            .collect();
        op
    }

    #[test]
    fn the_notes_and_the_steps_follow_in_the_order_they_came() {
        let mut op = op(&[("load model", 1_210), ("transcribe", 40_900)]);
        op.note("small model");
        op.note(format_args!("{} of sound", span(Duration::from_secs(134))));
        let mut line = String::new();
        op.detail(&mut line);
        assert_eq!(
            line,
            " · small model, 2 m 14 s of sound · load model 1.21 s · transcribe 40.9 s"
        );
        op.done("120 captions");
    }

    #[test]
    fn a_step_taken_twice_is_one_step_that_took_the_sum() {
        let mut op = op(&[]);
        op.step("decode");
        op.step("draw");
        op.step("decode");
        op.end_step();
        let names: Vec<&str> = op.steps.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["decode", "draw"]);
        op.cancelled();
    }

    #[test]
    fn a_result_ends_the_work_and_comes_back_as_it_was() {
        let fine: Result<u32, String> = op(&[]).end(Ok(7));
        assert_eq!(fine, Ok(7));
        let broken: Result<u32, String> = op(&[]).end(Err("no model".to_owned()));
        assert_eq!(broken, Err("no model".to_owned()));
    }
}
