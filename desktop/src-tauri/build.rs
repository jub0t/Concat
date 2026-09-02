// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

fn main() {
    // A release build is a packaged build, and a packaged app that shipped
    // without its FFmpeg pair has happened once already: everything media-
    // shaped failed at runtime, launched from Finder with no PATH to fall
    // back on. Fail the build instead - staging the binaries is part of
    // building for release.
    // A packager whose launcher guarantees the tools on the app's PATH - the
    // Nix wrapper puts ffmpeg, ffprobe and whisper-cli there - opts out
    // explicitly instead of staging copies it is not allowed to ship.
    println!("cargo:rerun-if-env-changed=WOLFCUT_SYSTEM_TOOLS");
    let system_tools = std::env::var_os("WOLFCUT_SYSTEM_TOOLS").is_some();

    if !system_tools && std::env::var("PROFILE").as_deref() == Ok("release") {
        let suffix = if cfg!(target_os = "windows") { ".exe" } else { "" };
        let staged = std::path::Path::new("ffmpeg");
        for binary in ["ffmpeg", "ffprobe"] {
            let path = staged.join(format!("{binary}{suffix}"));
            assert!(
                path.is_file(),
                "release build without a bundled {binary}: stage the FFmpeg pair in \
                 desktop/src-tauri/ffmpeg/ first, or the packaged app will ship broken"
            );
        }
        println!("cargo:rerun-if-changed=ffmpeg");

        // Same rule for the transcriber: users are never asked to install a
        // binary and point the app at it, so a release ships its own
        // whisper-cli or it does not ship.
        let whisper = std::path::Path::new("whisper").join(format!("whisper-cli{suffix}"));
        assert!(
            whisper.is_file(),
            "release build without a bundled whisper-cli: stage it in \
             desktop/src-tauri/whisper/ first (see the Build App workflow), or \
             transcription would arrive as a setup chore instead of a feature"
        );
        println!("cargo:rerun-if-changed=whisper");
    }

    tauri_build::build()
}
