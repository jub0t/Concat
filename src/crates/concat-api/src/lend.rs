// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The project on screen, lent to a caller of the API and given back.
//!
//! A window and the API it serves share one register of open projects, and
//! the register refuses the API a folder the window holds. An embedder that
//! sets a [`Lender`] lets the API ask for that folder instead: the window
//! saves and lets go, the API opens it as any other, and the window is told
//! when the API has let go again - on `project.close`, or when the API goes.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::{ApiError, ErrorCode, Holder, OpenProjects, key_of};

/// The window, as the API asks it for the project it has open.
pub trait Lender: Send + Sync {
    /// Asks the window to save `path` and let go of it in the register.
    /// Returns once it has, or why it would not.
    fn lend(&self, path: &str) -> Result<(), String>;

    /// The API has let go of `path`: the window may open it again.
    fn give_back(&self, path: &str);
}

/// The window an [`Api`](crate::Api) may borrow from, and what it has
/// borrowed.
#[derive(Default)]
pub(crate) struct Lending {
    lender: Option<Arc<dyn Lender>>,
    /// The borrowed folders, by the register's key.
    lent: BTreeSet<String>,
}

impl Lending {
    pub(crate) fn set_lender(&mut self, lender: Arc<dyn Lender>) {
        self.lender = Some(lender);
    }

    /// Asks the window for `path` when it holds it, before the API claims
    /// the folder. Nothing to do when nobody lends or the window does not
    /// hold it: the claim that follows says what it always said.
    pub(crate) fn borrow(&mut self, open: &OpenProjects, path: &str) -> Result<(), ApiError> {
        let Some(lender) = &self.lender else {
            return Ok(());
        };
        if open.holder(path) != Some(Holder::Window) {
            return Ok(());
        }
        let kept = |why: &str| {
            ApiError::new(
                ErrorCode::Refused,
                format!("{path} is open in the window, which did not lend it: {why}"),
            )
        };
        lender.lend(path).map_err(|why| kept(&why))?;
        if open.holder(path) == Some(Holder::Window) {
            return Err(kept("it still holds the project"));
        }
        self.lent.insert(key_of(path));
        Ok(())
    }

    /// The API has let go of `path`, or never got it: the window is told,
    /// once, if it lent it.
    pub(crate) fn returned(&mut self, path: &str) {
        if self.lent.remove(&key_of(path))
            && let Some(lender) = &self.lender
        {
            lender.give_back(path);
        }
    }
}

impl Drop for Lending {
    /// An API that goes with a borrowed project open - its server stopped -
    /// has let go of it in the register by now ([`Api`](crate::Api)'s own
    /// drop runs first), so the window takes it back.
    fn drop(&mut self) {
        if let Some(lender) = &self.lender {
            for key in &self.lent {
                lender.give_back(key);
            }
        }
    }
}
