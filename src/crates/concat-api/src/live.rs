// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Live mode: a caller edits the project open in the window while the
//! person at it watches, and every call lands in the window's own undo
//! history (#337).
//!
//! The window's [`Session`] lives on its event-loop thread and the API on
//! a thread of its own. A call on the window's project borrows the session
//! for exactly as long as the call takes: the window hands it over through
//! its [`LiveLink`] and holds its event loop until the [`Loan`] brings it
//! back. Nothing on screen reads a project that is not there, and nothing
//! on screen edits the project while the call does, so the two never
//! disagree about what the edit is. A call is short - the long work, an
//! export or a transcription, is a job that copies what it needs and
//! returns at once - so the window is held for the length of an edit.
//!
//! A loan always comes back. Returned by hand it says how the call left the
//! project ([`Returned`]); dropped - on an error, or while a panic unwinds -
//! it returns the session as it stands.
//!
//! Whether the window lends at all is its own setting, off unless the
//! person turns it on; a window that will not lend says why, and the call
//! fails with that.

use concat_host::Session;

use crate::ApiError;

/// How a loan came back: what the window does with the session next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Returned {
    /// The call saved the project: nothing on screen is unsaved any more.
    pub saved: bool,
}

/// The window's side of live mode: how the API borrows the session the
/// window has open. Implemented by the window, which is the only one that
/// knows whether it will lend and how to reach its own thread.
pub trait LiveLink: Send + Sync {
    /// Lends the window's session for the project folder at `path` until
    /// the loan is returned or dropped. Refused, with the reason the caller
    /// is told, when the window has another project open or none, when live
    /// mode is off, or when the person is in the middle of a gesture
    /// ([`crate::ErrorCode::Busy`]: try again in a moment).
    fn lend(&self, path: &str) -> Result<Loan, ApiError>;
}

/// The way a session goes home: called once, with the session and how
/// the call left it.
type GiveBack = Box<dyn FnOnce(Session, Returned) + Send>;

/// A session borrowed from the window for one call. Dropped, it gives the
/// session back as it stands; see the module docs.
pub struct Loan {
    session: Option<Session>,
    back: Option<GiveBack>,
}

impl Loan {
    /// A loan of `session`, which `back` takes home again.
    pub fn new(session: Session, back: impl FnOnce(Session, Returned) + Send + 'static) -> Loan {
        Loan {
            session: Some(session),
            back: Some(Box::new(back)),
        }
    }

    /// The session lent, for the length of the call. The loan then holds
    /// nothing, and [`Loan::give_back`] is how the session goes home.
    pub(crate) fn take(&mut self) -> Option<Session> {
        self.session.take()
    }

    /// Sends `session` home, saying how the call left it.
    pub(crate) fn give_back(mut self, session: Session, returned: Returned) {
        if let Some(back) = self.back.take() {
            back(session, returned);
        }
    }
}

impl Drop for Loan {
    fn drop(&mut self) {
        if let (Some(session), Some(back)) = (self.session.take(), self.back.take()) {
            back(session, Returned::default());
        }
    }
}
