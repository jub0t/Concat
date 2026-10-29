// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Downloading audio from a URL: a shell-out to `yt-dlp`, lite - one
//! binary dependency, no Rust crate for it. `yt-dlp` is not bundled;
//! the downloader says so when it is missing.
//! https://github.com/yt-dlp/yt-dlp

use std::path::{Path, PathBuf};
use std::process::Command;

/// Whether `yt-dlp` is on the PATH and can be used to download audio.
pub fn ytdl_available() -> bool {
    Command::new("yt-dlp").arg("--version").output().is_ok()
}

/// Download audio from `url` into `dir`, as MP3. Returns where `yt-dlp`
/// put it. Blocks: a worker's.
pub fn download_audio(url: &str, dir: &Path) -> Result<PathBuf, String> {
    if !ytdl_available() {
        return Err(
            "yt-dlp is not installed: install it from https://github.com/yt-dlp/yt-dlp \
             to download audio from a URL"
                .into(),
        );
    }
    std::fs::create_dir_all(dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    let template = dir.join("ytdl-%(title)s.%(ext)s");
    let output = Command::new("yt-dlp")
        .args([
            "-x",
            "--audio-format",
            "mp3",
            "--output",
            template.to_string_lossy().as_ref(),
            "--print",
            "filename",
            url,
        ])
        .output()
        .map_err(|error| format!("could not start yt-dlp: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("yt-dlp could not download {url}: {stderr}"));
    }
    let filename = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let path = PathBuf::from(filename);
    if !path.is_file() {
        return Err(format!("yt-dlp said it wrote {filename}, but nothing is there"));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// When `yt-dlp` is not on the PATH, `download_audio` refuses at once
    /// instead of starting a process that cannot exist.
    #[test]
    fn ytdl_absent_refuses_before_starting() {
        let dir = std::env::temp_dir();
        let result = download_audio("https://youtube.com/watch?v=nonexistent", &dir);
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(error.contains("yt-dlp is not installed") || error.contains("could not start"));
    }
}
