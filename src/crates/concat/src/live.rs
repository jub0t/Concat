// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The window's half of live mode: how the API on the Automation page's
//! socket borrows the project on screen for one call (#337).
//!
//! The API runs on a thread of its own and the session lives on the event
//! loop's. [`WindowLink::lend`] asks the event loop for the session; the
//! event loop hands it over and then waits, holding everything on screen
//! still, until the call sends it home - so for the length of one edit the
//! window draws nothing that could be out of date, and the person cannot
//! edit underneath the agent. The session's own history keeps every step
//! the call made, so each is undone like one made here.
//!
//! Whether the window lends is `prefs.server.live` (Settings > Automation),
//! read on the event loop at the moment of asking, so the switch takes
//! effect at once without restarting the server.

use std::sync::mpsc;
use std::time::Duration;

use concat_api::{ApiError, ErrorCode, LiveLink, Loan, Returned};
use concat_host::Session;

use crate::host::Shell;

/// How long a call waits for the event loop to answer before saying the
/// window is busy. Long enough for a frame or a dialog's work; short
/// enough that a window that is shutting down - waiting on this server
/// while the server waits on it - lets go.
const PATIENCE: Duration = Duration::from_secs(10);

/// The window as the API's [`LiveLink`]. Holds nothing: the window's state
/// is reached on its own thread, through [`Shell::with`].
pub struct WindowLink;

impl LiveLink for WindowLink {
    fn lend(&self, path: &str) -> Result<Loan, ApiError> {
        let (lent, lending) = mpsc::channel::<Result<Session, ApiError>>();
        let (home, coming_home) = mpsc::channel::<(Session, Returned)>();
        let wanted = path.to_owned();
        slint::invoke_from_event_loop(move || {
            Shell::with(|shell, app| {
                let asked = shell.studio.borrow_mut().lend_session(&wanted);
                let session = match asked {
                    Ok(session) => session,
                    Err(refusal) => {
                        let _ = lent.send(Err(refusal));
                        return;
                    }
                };
                // The call gave up waiting (PATIENCE): the session stays.
                if let Err(mpsc::SendError(Ok(session))) = lent.send(Ok(session)) {
                    shell
                        .studio
                        .borrow_mut()
                        .take_back(session, Returned::default());
                    return;
                }
                // Hold the event loop until it is home. A loan always
                // comes back, even from a call that failed or panicked
                // (see `concat_api::Loan`); a sender gone without it would
                // be a loan forgotten, which is said loudly.
                match coming_home.recv() {
                    Ok((session, returned)) => {
                        shell.studio.borrow_mut().take_back(session, returned);
                    }
                    Err(_) => {
                        log::error!(
                            "live: the project lent to a connected agent never came back; reopen it from the launch screen"
                        );
                        shell.studio.borrow_mut().notify(
                            &crate::i18n::t("settings.liveEditLost"),
                            true,
                        );
                    }
                }
                shell.studio.borrow().publish(&app, &shell.models);
            });
        })
        .map_err(|_| ApiError::new(ErrorCode::Busy, "the Concat window is closing"))?;
        let session = match lending.recv_timeout(PATIENCE) {
            Ok(answer) => answer?,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Err(ApiError::new(
                    ErrorCode::Busy,
                    "the Concat window is busy; try again in a moment",
                ));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(ApiError::new(
                    ErrorCode::Busy,
                    "the Concat window did not answer: it may be closing",
                ));
            }
        };
        Ok(Loan::new(session, move |session, returned| {
            let _ = home.send((session, returned));
        }))
    }
}
