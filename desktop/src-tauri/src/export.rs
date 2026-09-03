// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Tauri shim over the engine's export path.
//!
//! Rendering used to live in this file - transitions, timeline conversion,
//! the frame loop, the paused-monitor composite. All of it is
//! `concat-export` now, where the CLI and any other frontend render the
//! same way and the doctrine holds: the host converts wire formats and
//! reports progress, nothing more. What remains here is exactly that -
//! turning the engine's progress callback into `export://progress` events.

use std::sync::atomic::AtomicBool;

use serde::Serialize;
use tauri::Emitter;

pub use concat_export::{
    ClipKind, ExportClip, ExportRequest, PreviewFrameRequest, Reporter, TransitionSpec,
    preview_frame, preview_prefetch, render,
};

/// One progress event, as the UI's export dialog consumes it.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    frame: i64,
    total: i64,
    stage: &'static str,
}

/// Renders `request` and returns the path written. Tauri-facing wrapper that
/// reports through the `export://progress` event; the logic is in
/// [`concat_export::render`].
pub fn run(
    app: &tauri::AppHandle,
    request: ExportRequest,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let mut progress = |frame: i64, total: i64, stage: &'static str| {
        // A dropped progress event is not worth failing an export over.
        let _ = app.emit("export://progress", Progress { frame, total, stage });
    };
    render(&request, Reporter { progress: &mut progress, cancel })
}
