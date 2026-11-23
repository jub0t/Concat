// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Asking a job to stop, as a type.
//!
//! A cancel used to be recognised by finding the word "cancel" in an
//! error's text. The error for "one is already running - wait for it or
//! cancel it first" has that word in it, so it was taken for the person
//! pressing Cancel, and a sheet closed with nothing done. A
//! [`CancelToken`] is the question asked properly: a job checks it, and
//! the answer is never a string.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::lock;

/// Why a job was asked to stop.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CancelReason {
    /// Someone holding the job's token or its handle asked: the person
    /// pressed Cancel.
    Requested,
    /// Everything its owner had was cancelled: the project closed.
    Owner,
    /// A newer job with the same key took its place.
    Replaced,
    /// The scheduler is shutting down.
    Shutdown,
}

impl CancelReason {
    const fn code(self) -> u8 {
        match self {
            CancelReason::Requested => 1,
            CancelReason::Owner => 2,
            CancelReason::Replaced => 3,
            CancelReason::Shutdown => 4,
        }
    }

    const fn from_code(code: u8) -> Option<CancelReason> {
        match code {
            1 => Some(CancelReason::Requested),
            2 => Some(CancelReason::Owner),
            3 => Some(CancelReason::Replaced),
            4 => Some(CancelReason::Shutdown),
            _ => None,
        }
    }
}

impl fmt::Display for CancelReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            CancelReason::Requested => "cancelled on request",
            CancelReason::Owner => "cancelled with its owner",
            CancelReason::Replaced => "replaced by a newer job",
            CancelReason::Shutdown => "cancelled at shutdown",
        })
    }
}

/// What a check of a cancelled token returns: the job is to stop. It
/// carries nothing, on purpose; why is the token's to say
/// ([`CancelToken::reason`]).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Cancelled;

impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for Cancelled {}

/// What runs when a token is cancelled.
type Hook = Box<dyn FnOnce(CancelReason) + Send>;

/// A flag that says a job is to stop.
///
/// Cheap to clone - every clone is the same flag - and cheap to ask: a
/// check is one atomic load, fit for the inside of a loop that runs per
/// sample. Beyond the flag it can be waited on, which is how a job sleeps
/// without making its cancel wait for it ([`CancelToken::wait_timeout`]),
/// and it can call back, for work that is stopped by doing something - a
/// request aborted, a socket closed ([`CancelToken::on_cancel`]).
///
/// Cancelling is once and for good. The first reason given is the one
/// kept.
#[derive(Clone)]
pub struct CancelToken {
    inner: Arc<Inner>,
}

struct Inner {
    /// The flag itself, a plain `bool` so code that polls an
    /// `&AtomicBool` can be handed this one.
    flag: AtomicBool,
    /// The [`CancelReason`]'s code, or zero until there is one. Written
    /// before the flag, so a reader that sees the flag finds a reason.
    reason: AtomicU8,
    hooks: Mutex<Hooks>,
    /// Told when the flag is set, for the threads in a `wait`.
    changed: Condvar,
}

#[derive(Default)]
struct Hooks {
    waiting: Vec<Hook>,
    /// The job is over: nothing more is kept, see [`CancelToken::retire`].
    retired: bool,
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for CancelToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancelToken")
            .field("cancelled", &self.reason())
            .finish()
    }
}

impl CancelToken {
    /// A token nobody has cancelled.
    ///
    /// The scheduler makes one for every job it is given, and a job's
    /// token is only ever that job's: a stale cancel cannot stop the next
    /// run, and a new run cannot clear a cancel meant for the last. This
    /// is for code that takes a token and is run without a scheduler.
    pub fn new() -> CancelToken {
        CancelToken {
            inner: Arc::new(Inner {
                flag: AtomicBool::new(false),
                reason: AtomicU8::new(0),
                hooks: Mutex::new(Hooks::default()),
                changed: Condvar::new(),
            }),
        }
    }

    /// Whether the job is to stop. One atomic load.
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        self.inner.flag.load(Ordering::Acquire)
    }

    /// `Err(Cancelled)` once the job is to stop, for `?` in a loop.
    #[inline]
    pub fn check(&self) -> Result<(), Cancelled> {
        if self.is_cancelled() {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }

    /// Why the job is to stop, or `None` while it is not.
    pub fn reason(&self) -> Option<CancelReason> {
        if self.is_cancelled() {
            CancelReason::from_code(self.inner.reason.load(Ordering::Acquire))
        } else {
            None
        }
    }

    /// Asks the job to stop, as [`CancelReason::Requested`]. Returns
    /// whether this call was the one that did it.
    pub fn cancel(&self) -> bool {
        self.cancel_because(CancelReason::Requested)
    }

    /// Asks the job to stop for `reason`. Whoever was waiting is woken and
    /// every [`on_cancel`](CancelToken::on_cancel) is run, on this thread,
    /// before this returns.
    pub(crate) fn cancel_because(&self, reason: CancelReason) -> bool {
        let hooks = {
            let mut hooks = lock(&self.inner.hooks);
            if self.inner.flag.load(Ordering::Acquire) {
                return false;
            }
            self.inner.reason.store(reason.code(), Ordering::Release);
            self.inner.flag.store(true, Ordering::Release);
            // Under the lock a waiter holds between its look at the flag
            // and its sleep, so the wake-up cannot slip between the two.
            self.inner.changed.notify_all();
            std::mem::take(&mut hooks.waiting)
        };
        for hook in hooks {
            run_hook(hook, reason);
        }
        true
    }

    /// Blocks until the token is cancelled.
    pub fn wait(&self) {
        let mut hooks = lock(&self.inner.hooks);
        while !self.is_cancelled() {
            hooks = self
                .inner
                .changed
                .wait(hooks)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    /// Blocks until the token is cancelled or `timeout` has passed, and
    /// returns whether it was cancelled.
    ///
    /// This is a job's sleep: a download between two tries waits here and
    /// is gone the moment it is cancelled, where a plain sleep would hold
    /// the cancel up for as long as it had left to run.
    pub fn wait_timeout(&self, timeout: Duration) -> bool {
        let Some(deadline) = Instant::now().checked_add(timeout) else {
            self.wait();
            return true;
        };
        let mut hooks = lock(&self.inner.hooks);
        loop {
            if self.is_cancelled() {
                return true;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            hooks = self
                .inner
                .changed
                .wait_timeout(hooks, left)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }

    /// Runs `hook` when the token is cancelled, once, on the thread that
    /// cancels it - or now, on this one, if it already is.
    ///
    /// For work a flag cannot reach: the hook aborts the request or
    /// closes the socket the job is blocked in. It must be quick and must
    /// not wait on the job. A job's hooks that have not run by the time
    /// the job ends are dropped unrun.
    pub fn on_cancel(&self, hook: impl FnOnce(CancelReason) + Send + 'static) {
        let reason = {
            let mut hooks = lock(&self.inner.hooks);
            match self.reason() {
                Some(reason) => reason,
                None => {
                    if !hooks.retired {
                        hooks.waiting.push(Box::new(hook));
                    }
                    return;
                }
            }
        };
        run_hook(Box::new(hook), reason);
    }

    /// The flag as a plain `AtomicBool`, for code that polls one: a
    /// decoder's interrupt callback, a model's abort hook. For reading
    /// only - a store through it wakes nobody and runs no hook; cancel
    /// through the token.
    pub fn as_flag(&self) -> &AtomicBool {
        &self.inner.flag
    }

    /// Drops the hooks still waiting and keeps no new ones: the job this
    /// token was made for has ended, and a cancel from here on has nothing
    /// to stop. Without this a handle kept for good would keep every hook
    /// with it.
    pub(crate) fn retire(&self) {
        let dropped = {
            let mut hooks = lock(&self.inner.hooks);
            hooks.retired = true;
            std::mem::take(&mut hooks.waiting)
        };
        // Outside the lock: what a hook captured may have a drop of its
        // own that comes back to this token.
        drop(dropped);
    }
}

/// Runs one hook, keeping a panic in it from the thread that cancelled.
fn run_hook(hook: Hook, reason: CancelReason) {
    let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| hook(reason)));
    if ran.is_err() {
        log::error!("sched: a cancel hook panicked");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// Longer than anything here takes; a wait that reaches it has failed.
    const BOUND: Duration = Duration::from_secs(20);

    #[test]
    fn a_new_token_is_not_cancelled() {
        let token = CancelToken::new();
        assert!(!token.is_cancelled());
        assert_eq!(token.check(), Ok(()));
        assert_eq!(token.reason(), None);
        assert!(!token.as_flag().load(Ordering::Acquire));
    }

    #[test]
    fn every_clone_is_the_same_flag() {
        let token = CancelToken::new();
        let clone = token.clone();
        assert!(clone.cancel());
        assert!(token.is_cancelled());
        assert_eq!(token.check(), Err(Cancelled));
        assert_eq!(token.reason(), Some(CancelReason::Requested));
        assert!(token.as_flag().load(Ordering::Acquire));
    }

    #[test]
    fn the_first_reason_given_is_the_one_kept() {
        let token = CancelToken::new();
        assert!(token.cancel_because(CancelReason::Replaced));
        assert!(!token.cancel_because(CancelReason::Shutdown));
        assert!(!token.cancel());
        assert_eq!(token.reason(), Some(CancelReason::Replaced));
    }

    #[test]
    fn a_wait_that_nobody_cancels_ends_at_its_timeout() {
        let token = CancelToken::new();
        assert!(!token.wait_timeout(Duration::from_millis(5)));
        assert!(!token.wait_timeout(Duration::ZERO));
    }

    #[test]
    fn a_wait_on_a_cancelled_token_returns_at_once() {
        let token = CancelToken::new();
        token.cancel();
        assert!(token.wait_timeout(BOUND));
        token.wait();
    }

    #[test]
    fn a_thread_waiting_on_a_token_is_woken_by_its_cancel() {
        let token = CancelToken::new();
        let (waiting, is_waiting) = mpsc::channel();
        let waiter = {
            let token = token.clone();
            std::thread::spawn(move || {
                waiting.send(()).expect("the test is listening");
                token.wait_timeout(BOUND)
            })
        };
        is_waiting.recv_timeout(BOUND).expect("the waiter started");
        token.cancel();
        assert!(waiter.join().expect("the waiter did not panic"));
    }

    #[test]
    fn a_hook_runs_once_with_the_reason_when_the_token_is_cancelled() {
        let token = CancelToken::new();
        let (ran, runs) = mpsc::channel();
        token.on_cancel(move |reason| ran.send(reason).expect("the test is listening"));
        assert!(runs.try_recv().is_err());
        token.cancel_because(CancelReason::Owner);
        token.cancel();
        assert_eq!(runs.try_recv(), Ok(CancelReason::Owner));
        assert!(runs.try_recv().is_err());
    }

    #[test]
    fn a_hook_given_to_a_cancelled_token_runs_at_once() {
        let token = CancelToken::new();
        token.cancel();
        let (ran, runs) = mpsc::channel();
        token.on_cancel(move |reason| ran.send(reason).expect("the test is listening"));
        assert_eq!(runs.try_recv(), Ok(CancelReason::Requested));
    }

    #[test]
    fn a_retired_token_drops_its_hooks_unrun_and_keeps_no_more() {
        let token = CancelToken::new();
        let (ran, runs) = mpsc::channel::<()>();
        let late = ran.clone();
        token.on_cancel(move |_| ran.send(()).expect("the test is listening"));
        token.retire();
        token.on_cancel(move |_| late.send(()).expect("the test is listening"));
        // Both senders went with their hooks.
        assert_eq!(runs.try_recv(), Err(mpsc::TryRecvError::Disconnected));
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn a_hook_that_panics_does_not_keep_the_others_from_running() {
        let token = CancelToken::new();
        let (ran, runs) = mpsc::channel();
        token.on_cancel(|_| panic!("a hook that panics, for the test"));
        token.on_cancel(move |_| ran.send(()).expect("the test is listening"));
        assert!(token.cancel());
        assert_eq!(runs.try_recv(), Ok(()));
    }

    #[test]
    fn a_hook_may_ask_the_token_it_is_on() {
        let token = CancelToken::new();
        let (ran, runs) = mpsc::channel();
        let inside = token.clone();
        token.on_cancel(move |_| {
            ran.send((inside.is_cancelled(), inside.cancel()))
                .expect("the test is listening");
        });
        token.cancel();
        assert_eq!(runs.try_recv(), Ok((true, false)));
    }
}
