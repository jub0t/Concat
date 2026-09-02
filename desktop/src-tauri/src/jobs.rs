// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! One-at-a-time job slots for the host's long-running work.
//!
//! Export, transcription and model downloads are all "one at a time", and
//! that invariant used to live in doc comments while the commands happily
//! accepted a second run: two exports would race each other's temp files, and
//! starting a second job *cleared* the shared cancel flag out from under the
//! first. `SingleFlight` makes the invariant code: `begin` refuses a second
//! concurrent job, and every run gets its own cancel flag, so a cancel can
//! only ever stop the job that is actually running and a new job can never be
//! un-cancelled by a stale start or stopped by a stale cancel.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// The slot: at most one running job, identified by its own cancel flag.
pub struct SingleFlight {
    slot: Mutex<Option<Arc<AtomicBool>>>,
}

impl SingleFlight {
    pub fn new() -> Self {
        Self { slot: Mutex::new(None) }
    }

    /// Claims the slot for a new job, or reports the one already running.
    ///
    /// `what` is the user-facing job name ("export", "transcription", ...).
    pub fn begin(self: &Arc<Self>, what: &str) -> Result<Job, String> {
        let mut slot = self.slot.lock().map_err(|_| format!("{what} state poisoned"))?;
        if slot.is_some() {
            return Err(format!("a {what} is already running - wait for it or cancel it first"));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *slot = Some(Arc::clone(&cancel));
        Ok(Job { owner: Arc::clone(self), cancel })
    }

    /// Cancels the running job. Idle is a harmless no-op.
    pub fn cancel(&self) {
        if let Ok(slot) = self.slot.lock() {
            if let Some(cancel) = slot.as_ref() {
                cancel.store(true, Ordering::Relaxed);
            }
        }
    }
}

/// A claim on the slot, released on drop - however the job ended, panic
/// included, the slot frees and the next `begin` succeeds.
pub struct Job {
    owner: Arc<SingleFlight>,
    cancel: Arc<AtomicBool>,
}

impl Job {
    /// This run's stop flag, for the worker to poll.
    pub fn cancel_flag(&self) -> &AtomicBool {
        &self.cancel
    }

    /// An owned handle on the same flag, for callbacks that outlive borrows
    /// of the job (an FFI progress callback must be `'static`).
    pub fn cancel_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.owner.slot.lock() {
            // Release only our own claim. Guards are one-per-begin so the
            // check is belt and braces, but it makes a stale drop harmless.
            if slot.as_ref().is_some_and(|flag| Arc::ptr_eq(flag, &self.cancel)) {
                *slot = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_a_second_concurrent_job() {
        let flight = Arc::new(SingleFlight::new());
        let job = flight.begin("export").expect("first begin");
        match flight.begin("export") {
            Err(error) => assert!(error.contains("already running")),
            Ok(_) => panic!("second begin must be refused"),
        }
        drop(job);
        assert!(flight.begin("export").is_ok(), "slot frees on drop");
    }

    #[test]
    fn cancel_reaches_only_the_running_job() {
        let flight = Arc::new(SingleFlight::new());
        let first = flight.begin("export").expect("begin");
        flight.cancel();
        assert!(first.cancel_flag().load(Ordering::Relaxed));

        drop(first);
        // A new job starts with a fresh, uncancelled flag: the earlier cancel
        // cannot leak forward, and starting fresh cannot un-cancel anyone.
        let second = flight.begin("export").expect("begin again");
        assert!(!second.cancel_flag().load(Ordering::Relaxed));
    }

    #[test]
    fn cancel_when_idle_is_a_no_op() {
        let flight = Arc::new(SingleFlight::new());
        flight.cancel();
        assert!(flight.begin("export").is_ok());
    }
}
