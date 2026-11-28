// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors
//! The project on screen, lent to a caller of the window's API server.
//!
//! An agent that opens the project the window has open gets it: the window
//! saves and closes it, puts a sheet over itself that nothing gets past,
//! and opens the project again from the disk when the agent closes it. See
//! `concat_api::Lender` for the API's half.

use super::*;

/// How long the API's thread waits for the window to save and let go.
const PATIENCE: std::time::Duration = std::time::Duration::from_secs(10);

/// The window, as its API server asks it for the project on screen.
pub struct WindowLender;

impl concat_api::Lender for WindowLender {
    fn lend(&self, path: &str) -> Result<(), String> {
        let (answer, answered) = std::sync::mpsc::channel();
        let path = path.to_owned();
        crate::host::on_ui(move |studio, _, _| {
            // Nobody is listening after the wait below gave up; the window
            // has lent all the same and Take back is the way out.
            let _ = answer.send(studio.lend_project(&path));
        });
        answered
            .recv_timeout(PATIENCE)
            .unwrap_or_else(|_| Err("the window did not answer".to_owned()))
    }

    fn give_back(&self, path: &str) {
        let path = path.to_owned();
        crate::host::on_ui(move |studio, _, _| studio.take_back_project(&path));
    }
}

impl Studio {
    /// Saves and closes the open project for the API to open, when it is
    /// the one at `path`. A save that fails keeps it open and says why.
    fn lend_project(&mut self, path: &str) -> Result<(), String> {
        let open = self
            .session
            .as_ref()
            .map(|session| session.path().to_owned())
            .filter(|open| same_folder(open, path))
            .ok_or_else(|| "it is not the project on screen".to_owned())?;
        self.close_project()?;
        log::info!("project: lent {} to the Remote API", file_name_of(&open));
        self.lent = Some(open);
        Ok(())
    }

    /// Opens the lent project again, as the window knew it, once the API
    /// has let go of it. Told twice - the API's close and Take back - it
    /// opens once.
    fn take_back_project(&mut self, path: &str) {
        let Some(lent) = self.lent.take_if(|lent| same_folder(lent, path)) else {
            return;
        };
        if let Err(error) = self.open_project_at(&lent) {
            self.notify(&error, true);
            // The API kept it - a save of its own that failed: still lent.
            if self.host.open_projects.holder(&lent) == Some(concat_api::Holder::Api) {
                self.lent = Some(lent);
            }
        }
    }

    /// Take back on the sheet: the API saves and closes the project, which
    /// gives it back; with no server left, the window opens it itself.
    pub fn take_back_lent(&mut self) {
        let Some(lent) = self.lent.clone() else {
            return;
        };
        let Some(hub) = self.host.server.as_ref().map(|server| server.hub().clone()) else {
            self.take_back_project(&lent);
            return;
        };
        // Off the event loop: the API's thread may be in the middle of the
        // agent's call, and the close waits its turn behind it.
        let spawned = std::thread::Builder::new()
            .name("concat-take-back".to_owned())
            .spawn(move || {
                let closed = hub.call(concat_api::Request::ProjectClose {
                    path: lent.clone(),
                    save: true,
                });
                let concat_api::Response::Error(error) = closed else {
                    return;
                };
                crate::host::on_ui(move |studio, _, _| {
                    if error.code == concat_api::ErrorCode::NotOpen {
                        studio.take_back_project(&lent);
                    } else {
                        studio.notify(&error.message, true);
                    }
                });
            });
        if let Err(error) = spawned {
            log::error!("project: taking back the lent project could not start: {error}");
        }
    }

    /// The sheet over a window whose project is lent, as Slint shows it.
    pub(super) fn remote_edit_data(&self) -> crate::ui::RemoteEditData {
        match &self.lent {
            Some(lent) => crate::ui::RemoteEditData {
                open: true,
                message: lent_message(lent).into(),
            },
            None => crate::ui::RemoteEditData::default(),
        }
    }
}

/// What the sheet says, and what opening another project is refused with.
pub(super) fn lent_message(lent: &str) -> String {
    tf("studio.remoteEditMessage", &[&file_name_of(lent)])
}
