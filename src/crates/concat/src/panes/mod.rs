// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The window, one pane at a time.
//!
//! Each pane owns its state and is changed only by its own messages: a
//! Slint callback and a worker's report are the same thing, one [`Msg`]
//! posted to [`crate::studio::Studio::handle`], which routes it to the pane
//! and publishes. A message that arrives after the project it concerns
//! has closed is dropped there, in one place, rather than guarded against
//! in every closure.
//!
//! The panes move here one at a time from the window's controller; the
//! export sheet is the first, and the shape the rest follow.

pub mod captions;
pub mod export;
pub mod media_bin;
pub mod monitor;
pub mod project;
pub mod relink;
pub mod settings;
pub mod speech;
pub mod start;
pub mod timeline;

/// One thing that happened, to one pane.
#[derive(Debug)]
pub enum Msg {
    /// To the export sheet.
    Export(export::ExportMsg),
    /// To the settings sheet.
    Settings(settings::SettingsMsg),
    /// To the captions sheet.
    Captions(captions::CaptionsMsg),
    /// To the speech sheet.
    Speech(speech::SpeechMsg),
    /// To the missing media dialog.
    Relink(relink::RelinkMsg),
    /// To the project sheet.
    Project(project::ProjectMsg),
    /// To the launch screen's form.
    Start(start::StartMsg),
    /// To the media bin.
    Media(media_bin::MediaMsg),
    /// To the monitor.
    Monitor(monitor::MonitorMsg),
    /// To the timeline's view.
    Timeline(timeline::TimelineMsg),
}
