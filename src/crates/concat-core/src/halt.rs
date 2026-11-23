// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! How work that can be told to stop says it stopped.
//!
//! A cancel used to be an error like any other, and was told from a
//! failure by finding the word "cancelled" in its text. A file that would
//! not decode under a folder named `cancelled-takes` has that word in its
//! error too, so it was taken for the person pressing Cancel and nothing
//! was said about it. [`Halt`] keeps the two apart by what they are, and
//! nobody has to read the words.

use std::fmt;

/// Why work that can be told to stop ended without its result.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Halt {
    /// Whoever asked for the work told it to stop. Not a failure: there
    /// is nothing to tell the person that they do not already know.
    Cancelled,
    /// The work failed, and this is what went wrong, in words for the
    /// person or the log.
    Failed(String),
}

impl Halt {
    /// Whether the work was told to stop, rather than failing.
    pub const fn is_cancelled(&self) -> bool {
        matches!(self, Halt::Cancelled)
    }

    /// What went wrong, or `None` for a cancel.
    pub fn failure(&self) -> Option<&str> {
        match self {
            Halt::Cancelled => None,
            Halt::Failed(message) => Some(message),
        }
    }

    /// What went wrong, by value, or `None` for a cancel: for a caller
    /// that shows a failure and says nothing of a cancel.
    pub fn into_failure(self) -> Option<String> {
        match self {
            Halt::Cancelled => None,
            Halt::Failed(message) => Some(message),
        }
    }
}

/// The failure's own text, or "cancelled": for a log line, or a boundary
/// that has only text to give and says separately which of the two it was.
/// Nothing is to read it back to find out.
impl fmt::Display for Halt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Halt::Cancelled => f.write_str("cancelled"),
            Halt::Failed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for Halt {}

/// A text error is a failure, whatever its words: `?` on a step that
/// cannot be cancelled keeps working inside a job that can.
impl From<String> for Halt {
    fn from(message: String) -> Halt {
        Halt::Failed(message)
    }
}

impl From<&str> for Halt {
    fn from(message: &str) -> Halt {
        Halt::Failed(message.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_that_mentions_the_word_cancelled_is_still_a_failure() {
        let halt = Halt::from("could not decode /takes/cancelled/clip.mp4".to_owned());
        assert!(!halt.is_cancelled());
        assert_eq!(
            halt.failure(),
            Some("could not decode /takes/cancelled/clip.mp4")
        );
        assert_eq!(
            Halt::from("cancelled"),
            Halt::Failed("cancelled".to_owned())
        );
    }

    #[test]
    fn a_cancel_has_no_failure_to_show() {
        assert!(Halt::Cancelled.is_cancelled());
        assert_eq!(Halt::Cancelled.failure(), None);
        assert_eq!(Halt::Cancelled.into_failure(), None);
        assert_eq!(Halt::Cancelled.to_string(), "cancelled");
    }

    #[test]
    fn a_failure_reads_as_its_own_text() {
        let halt = Halt::from("the disk is full");
        assert_eq!(halt.to_string(), "the disk is full");
        assert_eq!(halt.into_failure().as_deref(), Some("the disk is full"));
    }

    #[test]
    fn a_text_error_passes_through_a_question_mark_as_a_failure() {
        fn step() -> Result<(), String> {
            Err("download cancelled by the server".to_owned())
        }
        fn job() -> Result<(), Halt> {
            step()?;
            Ok(())
        }
        assert_eq!(
            job(),
            Err(Halt::Failed("download cancelled by the server".to_owned()))
        );
    }
}
