// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Live mode (#337): the project on screen, lent to the API's server.
//!
//! The server runs the API on a thread of its own; the window's state never
//! leaves the event-loop thread. So a request naming the project on screen
//! is not run where it arrives: the server's thread hands it to the event
//! loop with `slint::invoke_from_event_loop`, the window runs it against
//! its own session the way one of its own edits runs - one undo step, the
//! caches and the monitor brought up to date, the state published - and
//! the answer goes back over a channel while the server's thread waits.
//!
//! The window never waits on the server, so the wait cannot turn around
//! into a deadlock; it is bounded all the same, so a window that is
//! closing, or stuck, costs a caller [`PATIENCE`] and an error rather than
//! the server's thread.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use concat_api::{ApiError, ErrorCode, WindowLink};
use concat_host::Session;
use concat_host::session::EditorView;
use concat_project::Command;
use concat_project::model::VideoSettings;

use crate::host::Shell;
use crate::studio::{Studio, same_folder};

/// How long a caller waits for the window to answer.
const PATIENCE: Duration = Duration::from_secs(10);

/// The window's side of [`WindowLink`], made for each server the
/// Automation page starts.
pub struct Live {
    /// The Automation page's switch, shared with the window.
    switch: Arc<AtomicBool>,
}

impl Live {
    /// The window's project, lent while `switch` is on.
    pub fn new(switch: Arc<AtomicBool>) -> Live {
        Live { switch }
    }

    /// Reads the window's state; nothing is published.
    fn read<T: Send + 'static>(
        &self,
        path: &str,
        body: impl FnOnce(&Studio) -> Result<T, ApiError> + Send + 'static,
    ) -> Result<T, ApiError> {
        self.on_window(path, false, |studio| body(studio))
    }

    /// Changes the window's state, and publishes it, so the change is on
    /// screen as the caller hears it landed.
    fn change<T: Send + 'static>(
        &self,
        path: &str,
        body: impl FnOnce(&mut Studio) -> Result<T, ApiError> + Send + 'static,
    ) -> Result<T, ApiError> {
        self.on_window(path, true, body)
    }

    /// One step of the window's history, back or forward.
    fn step(&self, path: &str, step: fn(&mut Studio)) -> Result<EditorView, ApiError> {
        self.change(path, move |studio| {
            step(studio);
            studio.note_script_edit();
            Ok(session(studio)?.view())
        })
    }

    /// Runs `body` on the event loop against the window's state, when the
    /// window still has `path` open and the switch is still on by the time
    /// it gets there, and waits for the answer.
    fn on_window<T: Send + 'static>(
        &self,
        path: &str,
        publish: bool,
        body: impl FnOnce(&mut Studio) -> Result<T, ApiError> + Send + 'static,
    ) -> Result<T, ApiError> {
        let (reply, answer) = mpsc::channel();
        let switch = Arc::clone(&self.switch);
        let path = path.to_owned();
        slint::invoke_from_event_loop(move || {
            let mut outcome = None;
            Shell::with(|shell, app| {
                let result = {
                    let mut studio = shell.studio.borrow_mut();
                    let open = studio
                        .session
                        .as_ref()
                        .is_some_and(|session| same_folder(session.path(), &path));
                    if open && switch.load(Ordering::Relaxed) {
                        body(&mut studio)
                    } else {
                        Err(not_open(&path))
                    }
                };
                if publish && result.is_ok() {
                    shell.studio.borrow().publish(&app, &shell.models);
                }
                outcome = Some(result);
            });
            // The receiver gone means the caller stopped waiting; nobody
            // is left to tell.
            let _ = reply.send(outcome.unwrap_or_else(|| Err(closing())));
        })
        .map_err(|_| closing())?;
        match answer.recv_timeout(PATIENCE) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => Err(ApiError::failed(format!(
                "the window did not answer within {} s",
                PATIENCE.as_secs()
            ))),
            Err(RecvTimeoutError::Disconnected) => Err(closing()),
        }
    }
}

impl WindowLink for Live {
    fn live(&self) -> bool {
        self.switch.load(Ordering::Relaxed)
    }

    fn view(&self, path: &str) -> Result<EditorView, ApiError> {
        self.read(path, |studio| Ok(session(studio)?.view()))
    }

    fn snapshot(&self, path: &str) -> Result<Session, ApiError> {
        self.read(path, |studio| Ok(session(studio)?.snapshot()))
    }

    fn apply(&self, path: &str, command: Command) -> Result<EditorView, ApiError> {
        self.change(path, move |studio| apply_as_script(studio, command))
    }

    fn set_video(&self, path: &str, video: VideoSettings) -> Result<EditorView, ApiError> {
        // The timeline on screen, as the window's own frame setting picks
        // it, through the same edit and its HDR notice.
        self.change(path, move |studio| {
            let timeline_id = session(studio)?.project().active_timeline_id.clone();
            apply_as_script(studio, Command::SetTimelineVideo { timeline_id, video })
        })
    }

    fn undo(&self, path: &str) -> Result<EditorView, ApiError> {
        self.step(path, Studio::undo)
    }

    fn redo(&self, path: &str) -> Result<EditorView, ApiError> {
        self.step(path, Studio::redo)
    }

    fn save(&self, path: &str) -> Result<(), ApiError> {
        // The window's save writes on a worker and says so if it fails; the
        // caller hears that it was asked for.
        self.change(path, |studio| {
            studio.save(false);
            Ok(())
        })
    }
}

/// An edit a script asked for, run as the window runs its own, with the
/// edit layer's no handed back to the script rather than shown.
fn apply_as_script(studio: &mut Studio, command: Command) -> Result<EditorView, ApiError> {
    let view = studio
        .try_apply(command)
        .ok_or_else(no_project)?
        .map_err(|message| ApiError::new(ErrorCode::Refused, message))?;
    studio.note_script_edit();
    Ok(view)
}

fn session(studio: &Studio) -> Result<&Session, ApiError> {
    studio.session.as_ref().ok_or_else(no_project)
}

fn not_open(path: &str) -> ApiError {
    ApiError::new(
        ErrorCode::NotOpen,
        format!("{path} is no longer open in the window, or no longer lent to scripts"),
    )
}

fn no_project() -> ApiError {
    ApiError::failed("the window has no project open")
}

fn closing() -> ApiError {
    ApiError::failed("the window is closing")
}
