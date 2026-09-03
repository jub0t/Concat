// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Where the FFmpeg binaries are.
//!
//! By default they are looked up on `PATH`, which is what a developer with
//! FFmpeg installed expects. An application that ships its own copy calls
//! [`set_binaries`] once at startup and everything downstream picks it up,
//! because every process this crate spawns goes through [`ffmpeg`] or
//! [`ffprobe`] rather than naming the program itself.
//!
//! Set once, at startup, before any decoding. Later calls are ignored rather
//! than racing: a half-switched pair - old encoder, new decoder - would be a
//! genuinely confusing thing to debug.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// One cell for the pair, so two racing first-callers can never produce a
/// mixed set - the decoder from one install, the prober from another.
static BINARIES: OnceLock<(PathBuf, PathBuf)> = OnceLock::new();

/// Points the crate at a specific pair of binaries.
///
/// Returns `false` if they were already resolved, in which case nothing
/// changed. Call this before opening any media.
pub fn set_binaries(ffmpeg: impl Into<PathBuf>, ffprobe: impl Into<PathBuf>) -> bool {
    BINARIES.set((ffmpeg.into(), ffprobe.into())).is_ok()
}

fn resolved() -> &'static (PathBuf, PathBuf) {
    BINARIES.get_or_init(|| (PathBuf::from("ffmpeg"), PathBuf::from("ffprobe")))
}

/// The `ffmpeg` binary to run. Defaults to whatever `PATH` finds.
pub fn ffmpeg() -> &'static Path {
    &resolved().0
}

/// The `ffprobe` binary to run. Defaults to whatever `PATH` finds.
pub fn ffprobe() -> &'static Path {
    &resolved().1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_the_bare_name_so_path_resolves_it() {
        // Deliberately does not call set_binaries: these statics are global,
        // and a test that set them would change the behaviour of every other
        // test in the binary depending on the order they happened to run in.
        assert_eq!(ffmpeg().as_os_str(), "ffmpeg");
        assert_eq!(ffprobe().as_os_str(), "ffprobe");
    }
}
