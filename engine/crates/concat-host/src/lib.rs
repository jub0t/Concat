// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! What the editor window needs that is not the edit itself.
//!
//! The edit lives in `concat-project`, pixels in `concat-render` and
//! `concat-export`, files in `concat-media`. This crate is the layer the
//! window talks to: it opens a project folder as a [`session::Session`],
//! keeps the recents list, caches waveforms and filmstrips beside the
//! project, composites the paused monitor's true frame, plays the audible
//! clips, packs templates, and holds the one-at-a-time slots for long jobs.
//!
//! It began as the host process of an earlier, web-based front end, where
//! every one of these converted wire formats. There is no wire - the window
//! calls these functions in-process - so what is left is the plumbing
//! itself, with the same doctrine: no editing decision is made here. If a
//! function starts deciding what an edit means, it belongs in
//! `concat-project`.
//!
//! Nothing here knows about a window. Long work reports through callbacks
//! and cancels through flags, and the caller decides which thread it runs on.

pub mod dirs;
pub mod export;
pub mod jobs;
pub mod media;
pub mod playback;
pub mod preview;
pub mod projects;
pub mod session;
pub mod templates;

pub use dirs::AppDirs;
pub use jobs::{Job, SingleFlight};
pub use projects::ProjectInfo;
pub use session::{EditorView, Session, SettingsView};
