// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The bottom-right notice's state: which one is up, how long it holds, and
//! which notice may take its place.
//!
//! The window owns the timing (it pauses while the pointer rests on the
//! card), so it says when a notice is gone; this owns the rule about what
//! may replace one, because that is state and wants testing without a
//! window.

use crate::ui::ToastKind;

/// How long a confirmation stays: a glance.
const DONE_HOLD_MS: i64 = 4_000;
/// The least a warning or a failure stays: long enough to be read and acted
/// on, which is what its row of actions is for.
const PROBLEM_HOLD_MS: i64 = 10_000;
/// The most it stays without the pointer on it. Past this a notice is a
/// page, and the log is where a page belongs.
const PROBLEM_HOLD_CAP_MS: i64 = 20_000;
/// Characters read in the least hold; every 15 past them buy a second.
const PROBLEM_READ_CHARS: i64 = 60;
const CHARS_PER_SECOND: i64 = 15;

/// The bottom-right notice: one at a time. The token is what the panel
/// watches, bumped per notice so the same sentence said twice blinks twice.
#[derive(Default)]
pub struct ToastState {
    pub token: i32,
    pub message: String,
    pub kind: ToastKind,
    /// How long the window holds it, pointer away; see [`hold_ms`].
    pub hold_ms: i64,
    /// Between a notice being raised and the window saying it went, by
    /// running out or by its close button.
    pub up: bool,
}

impl ToastState {
    /// Puts `message` up, unless a failure still up outranks it. Returns
    /// whether it was raised.
    ///
    /// A confirmation arriving over a failure is dropped rather than shown:
    /// an autosave's "Project saved" a second after an export failed would
    /// otherwise wipe the one notice that mattered, and the log already has
    /// the confirmation. A warning or a failure always takes the card - the
    /// newer problem is the one worth reading.
    pub fn raise(&mut self, message: &str, kind: ToastKind) -> bool {
        if kind == ToastKind::Done && self.up && self.kind == ToastKind::Failed {
            log::debug!("notice: kept the failure up over \"{message}\"");
            return false;
        }
        self.token = self.token.wrapping_add(1);
        self.message = message.into();
        self.kind = kind;
        self.hold_ms = hold_ms(kind, message);
        self.up = true;
        true
    }

    /// The window says notice `token` has gone. One it has already replaced
    /// is not the one up, and changes nothing.
    pub fn gone(&mut self, token: i32) {
        if token == self.token {
            self.up = false;
        }
    }
}

/// How long a notice holds with the pointer away from it. A confirmation
/// is a glance; a problem is read, so it holds longer the more there is to
/// read, up to a cap.
pub fn hold_ms(kind: ToastKind, message: &str) -> i64 {
    match kind {
        ToastKind::Done => DONE_HOLD_MS,
        ToastKind::Warning | ToastKind::Failed => {
            let chars = message.chars().count() as i64;
            let extra = (chars - PROBLEM_READ_CHARS).max(0) * 1_000 / CHARS_PER_SECOND;
            (PROBLEM_HOLD_MS + extra).min(PROBLEM_HOLD_CAP_MS)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "Project saved" from an autosave a second after a failure leaves the
    /// failure up, and says nothing new.
    #[test]
    fn a_confirmation_does_not_replace_a_failure_still_up() {
        let mut toast = ToastState::default();
        assert!(toast.raise("Could not export: the disk is full", ToastKind::Failed));
        let token = toast.token;
        assert!(!toast.raise("Project saved", ToastKind::Done));
        assert_eq!(toast.token, token);
        assert_eq!(toast.kind, ToastKind::Failed);
        assert_eq!(toast.message, "Could not export: the disk is full");
    }

    /// Once the failure has gone, a confirmation is shown as usual.
    #[test]
    fn a_confirmation_follows_a_failure_once_it_has_gone() {
        let mut toast = ToastState::default();
        toast.raise("Could not export", ToastKind::Failed);
        toast.gone(toast.token);
        assert!(toast.raise("Project saved", ToastKind::Done));
        assert_eq!(toast.kind, ToastKind::Done);
    }

    /// A newer problem takes the card from an older one, of either kind.
    #[test]
    fn a_newer_problem_replaces_an_older_one() {
        let mut toast = ToastState::default();
        toast.raise("Could not export", ToastKind::Failed);
        assert!(toast.raise("Could not save", ToastKind::Failed));
        assert_eq!(toast.message, "Could not save");
        assert!(toast.raise("Drawing the slow way", ToastKind::Warning));
        assert_eq!(toast.kind, ToastKind::Warning);
        assert!(toast.raise("Could not open", ToastKind::Failed));
        assert_eq!(toast.kind, ToastKind::Failed);
    }

    /// Confirmations replace each other, and a warning, as before.
    #[test]
    fn a_confirmation_replaces_a_confirmation_or_a_warning() {
        let mut toast = ToastState::default();
        toast.raise("Project saved", ToastKind::Done);
        let token = toast.token;
        assert!(toast.raise("Project saved", ToastKind::Done));
        assert_ne!(toast.token, token, "the same sentence twice is two notices");
        toast.raise("Drawing the slow way", ToastKind::Warning);
        assert!(toast.raise("Project saved", ToastKind::Done));
    }

    /// The window reporting a notice it has already replaced does not take
    /// the newer one down.
    #[test]
    fn a_stale_gone_leaves_the_newer_failure_up() {
        let mut toast = ToastState::default();
        toast.raise("Could not export", ToastKind::Failed);
        let old = toast.token;
        toast.raise("Could not save", ToastKind::Failed);
        toast.gone(old);
        assert!(!toast.raise("Project saved", ToastKind::Done));
    }

    #[test]
    fn holds_are_a_glance_for_done_and_grow_with_a_problems_length() {
        assert_eq!(hold_ms(ToastKind::Done, &"x".repeat(400)), 4_000);
        assert_eq!(hold_ms(ToastKind::Failed, "short"), 10_000);
        assert_eq!(hold_ms(ToastKind::Warning, &"x".repeat(60)), 10_000);
        assert_eq!(hold_ms(ToastKind::Failed, &"x".repeat(90)), 12_000);
        assert_eq!(hold_ms(ToastKind::Failed, &"x".repeat(210)), 20_000);
        assert_eq!(hold_ms(ToastKind::Failed, &"x".repeat(2_000)), 20_000);
        // Characters, not bytes: a sentence in Bengali is not held three
        // times as long as the same sentence in English.
        assert_eq!(hold_ms(ToastKind::Failed, &"অ".repeat(90)), 12_000);
    }
}
