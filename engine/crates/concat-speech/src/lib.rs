// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Speech in and out, entirely on this machine.
//!
//! - [`transcribe`] - whisper.cpp, in-process, turns a clip's audio into
//!   timed caption segments.
//! - [`tts`] - Kokoro, through sherpa-onnx, turns typed narration into a
//!   WAV in the project folder.
//!
//! Both download their models on demand into the app's data directory and
//! never bundle them. Both are one-at-a-time: a [`concat_host::SingleFlight`]
//! refuses a second concurrent run rather than letting two share a cancel
//! flag.
//!
//! A separate crate from `concat-host` so the heavy native dependency
//! (sherpa-onnx's static libraries) stays out of everything that does not
//! speak.

pub mod transcribe;
pub mod tts;

pub use transcribe::Transcriber;
pub use tts::Speech;

/// Progress for one model download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadProgress {
    /// Which model.
    pub id: String,
    /// Bytes received so far.
    pub received: u64,
    /// Content-Length when the server sent one, the table estimate otherwise.
    pub total: u64,
    /// True while an archive is being unpacked - bytes stop moving but the
    /// job is far from done, and the bar should say so.
    pub unpacking: bool,
    /// True on the final report.
    pub done: bool,
}

/// Streams `url` into `partial`, reporting every couple of megabytes and
/// stopping when `cancel` is set. Shared by both model downloaders.
fn download_to(
    url: &str,
    partial: &std::path::Path,
    id: &str,
    estimate: u64,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &mut dyn FnMut(DownloadProgress),
) -> Result<(u64, u64), String> {
    use std::io::Read;
    use std::sync::atomic::Ordering;

    // A read timeout, or a stalled connection blocks in `read` forever
    // with the cancel flag unreachable - the flag is only checked between
    // reads. Thirty seconds without a byte means the download is dead.
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(20))
        .timeout_read(std::time::Duration::from_secs(30))
        .build();
    let response = agent
        .get(url)
        .call()
        .map_err(|error| format!("download failed: {error}"))?;
    let total = response
        .header("Content-Length")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(estimate);

    let mut file = std::fs::File::create(partial)
        .map_err(|error| format!("could not create {}: {error}", partial.display()))?;

    let mut reader = response.into_reader();
    let mut buffer = [0u8; 64 * 1024];
    let mut received: u64 = 0;
    let mut last_report: u64 = 0;

    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(file);
            let _ = std::fs::remove_file(partial);
            return Err("download cancelled".to_owned());
        }
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("download interrupted: {error}"))?;
        if read == 0 {
            break;
        }
        std::io::Write::write_all(&mut file, &buffer[..read])
            .map_err(|error| format!("could not write model: {error}"))?;
        received += read as u64;

        // Every 2 MB, not every chunk: a progress bar cannot use more.
        if received - last_report >= 2 * 1024 * 1024 {
            last_report = received;
            progress(DownloadProgress {
                id: id.to_owned(),
                received,
                total,
                unpacking: false,
                done: false,
            });
        }
    }
    Ok((received, received.max(total)))
}
