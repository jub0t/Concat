// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The inputs that failed, so they are not tried again on every edit.
//!
//! A cutout whose model could not be fetched used to be started again by
//! every edit, each time with the same download and the same toast. A job
//! may now say what it was run on - an [`InputKey`] - and when it fails
//! the failure is kept under that key. The next job with the same key is
//! answered with the failure it would have had, without running, until
//! the key is forgotten or the input - and so its key - changes.

use std::any::Any;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use crate::job::InputKey;

/// A job's own error, kept without its type; the job that asks for it
/// back knows what it was.
pub(crate) type Kept = Arc<dyn Any + Send + Sync>;

/// The failures kept, newest last, and no more than `limit` of them: when
/// one more arrives the oldest goes. An input forgotten that way is simply
/// tried again, which is what would have happened without this.
pub(crate) struct FailureMemory {
    limit: usize,
    kept: HashMap<InputKey, Kept>,
    /// The keys in the order they were first kept.
    order: VecDeque<InputKey>,
}

impl FailureMemory {
    pub(crate) fn new(limit: usize) -> FailureMemory {
        FailureMemory {
            limit,
            kept: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    /// Keeps `error` as what `key` fails with. A key already kept has its
    /// error replaced and keeps its place in the line.
    pub(crate) fn remember(&mut self, key: InputKey, error: Kept) {
        if self.limit == 0 {
            return;
        }
        if let Some(kept) = self.kept.get_mut(&key) {
            *kept = error;
            return;
        }
        while self.kept.len() >= self.limit {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.kept.remove(&oldest);
        }
        self.order.push_back(key.clone());
        self.kept.insert(key, error);
    }

    /// What `key` failed with, if it did.
    pub(crate) fn recall(&self, key: &InputKey) -> Option<Kept> {
        self.kept.get(key).cloned()
    }

    /// Forgets `key`'s failure. Returns whether there was one.
    pub(crate) fn forget(&mut self, key: &InputKey) -> bool {
        if self.kept.remove(key).is_none() {
            return false;
        }
        self.order.retain(|kept| kept != key);
        true
    }

    /// Forgets every failure.
    pub(crate) fn clear(&mut self) {
        self.kept.clear();
        self.order.clear();
    }

    /// How many failures are kept.
    pub(crate) fn len(&self) -> usize {
        self.kept.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str) -> InputKey {
        InputKey::new(name)
    }

    fn error(text: &'static str) -> Kept {
        Arc::new(text)
    }

    fn text(kept: Option<Kept>) -> Option<&'static str> {
        kept.and_then(|kept| kept.downcast_ref::<&'static str>().copied())
    }

    #[test]
    fn a_failure_is_kept_under_its_key_until_it_is_forgotten() {
        let mut memory = FailureMemory::new(8);
        assert_eq!(text(memory.recall(&key("a"))), None);
        memory.remember(key("a"), error("offline"));
        assert_eq!(text(memory.recall(&key("a"))), Some("offline"));
        assert_eq!(text(memory.recall(&key("b"))), None);
        assert!(memory.forget(&key("a")));
        assert!(!memory.forget(&key("a")));
        assert_eq!(text(memory.recall(&key("a"))), None);
        assert_eq!(memory.len(), 0);
    }

    #[test]
    fn one_more_failure_than_the_limit_drops_the_oldest() {
        let mut memory = FailureMemory::new(2);
        memory.remember(key("a"), error("first"));
        memory.remember(key("b"), error("second"));
        memory.remember(key("c"), error("third"));
        assert_eq!(memory.len(), 2);
        assert_eq!(text(memory.recall(&key("a"))), None);
        assert_eq!(text(memory.recall(&key("b"))), Some("second"));
        assert_eq!(text(memory.recall(&key("c"))), Some("third"));
    }

    #[test]
    fn a_key_that_fails_again_keeps_its_place_and_its_newest_error() {
        let mut memory = FailureMemory::new(2);
        memory.remember(key("a"), error("first"));
        memory.remember(key("b"), error("second"));
        memory.remember(key("a"), error("again"));
        assert_eq!(memory.len(), 2);
        memory.remember(key("c"), error("third"));
        // `a` was still the oldest.
        assert_eq!(text(memory.recall(&key("a"))), None);
        assert_eq!(text(memory.recall(&key("b"))), Some("second"));
    }

    #[test]
    fn a_forgotten_key_leaves_no_gap_in_the_line() {
        let mut memory = FailureMemory::new(2);
        memory.remember(key("a"), error("first"));
        memory.remember(key("b"), error("second"));
        memory.forget(&key("a"));
        memory.remember(key("c"), error("third"));
        memory.remember(key("d"), error("fourth"));
        assert_eq!(text(memory.recall(&key("b"))), None);
        assert_eq!(text(memory.recall(&key("c"))), Some("third"));
        assert_eq!(text(memory.recall(&key("d"))), Some("fourth"));
    }

    #[test]
    fn a_memory_of_nothing_keeps_nothing() {
        let mut memory = FailureMemory::new(0);
        memory.remember(key("a"), error("first"));
        assert_eq!(memory.len(), 0);
        memory.clear();
        assert_eq!(text(memory.recall(&key("a"))), None);
    }
}
