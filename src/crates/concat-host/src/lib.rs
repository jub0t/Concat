// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! What the editor window needs that is not the edit itself.
//!
//! The edit lives in `concat-project`, pixels in `concat-render` and
//! `concat-export`, files in `concat-media`. This crate is the layer the
//! window talks to: it opens a project folder as a [`session::Session`],
//! keeps the recents list, caches waveforms and filmstrips beside the
//! project, composites the paused monitor's true frame, plays the audible
//! clips, finds the masks behind cutouts, packs templates, and holds the
//! one-at-a-time slots for long jobs.
//!
//! The window calls these functions in-process, and the doctrine is that
//! no editing decision is made here. If a function starts deciding what an
//! edit means, it belongs in `concat-project`.
//!
//! Nothing here knows about a window. Long work reports through callbacks
//! and cancels through flags, and the caller decides which thread it runs on.

pub mod brush;
pub mod cutout;
pub mod dirs;
pub mod enhance;
pub mod export;
pub mod jobs;
pub mod logs;
pub mod media;
pub mod models;
pub mod playback;
pub mod preview;
pub mod projects;
pub mod proxy;
pub mod reverse;
pub mod session;
pub mod templates;
pub mod titles;

pub use brush::{Brushes, RegionRequest};
pub use cutout::{AnalyseRequest, Cutouts};
pub use dirs::AppDirs;
pub use enhance::{EnhanceRequest, Enhancers};
pub use jobs::{Job, SingleFlight};
pub use projects::ProjectInfo;
pub use reverse::{ReverseRequest, Reversers};
pub use session::{EditorView, Session, SettingsView};
pub use titles::{TitleClip, Titles};

/// The one scheduler every decode for the screen goes through: the
/// monitor's frames and the frames ahead of the playhead first, then
/// filmstrips, the bin's artwork and proxies in turn, on a few threads
/// with one always kept clear of the background work. Owns the reader
/// pool the monitors read; see `concat_media::Prefetcher`.
pub fn scheduler() -> &'static std::sync::Arc<concat_media::Prefetcher> {
    static SCHEDULER: std::sync::OnceLock<std::sync::Arc<concat_media::Prefetcher>> =
        std::sync::OnceLock::new();
    SCHEDULER.get_or_init(|| {
        std::sync::Arc::new(concat_media::Prefetcher::with_defaults(
            std::sync::Arc::new(concat_media::ReaderPool::with_defaults()),
        ))
    })
}
