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

#[cfg(feature = "chatterbox")]
pub mod chatterbox;
pub mod transcribe;
pub mod tts;

pub use transcribe::Transcriber;
pub use tts::Speech;

/// Whether the voices run on the machine's own accelerator where the build
/// has one, or on the CPU. Process-wide, the way `concat_media`'s hardware
/// decode preference is: Settings › Speech sets it, and an engine reads it
/// as it loads - one already loaded the other way is loaded again on the
/// next read. Off until asked, because the accelerator is a bet: CoreML
/// runs the parts of a network it knows and hands the rest back to the
/// CPU, and which parts those are is the model's business.
static ACCELERATED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Asks the engines to run on the accelerator, or not to.
pub fn set_accelerated(on: bool) {
    ACCELERATED.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Whether the engines are asked to run on the accelerator.
pub fn accelerated() -> bool {
    ACCELERATED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Whether this build has an accelerator to offer at all: CoreML, on a
/// Mac. Elsewhere the switch is not shown, since it would do nothing.
pub const fn acceleration_offered() -> bool {
    cfg!(target_os = "macos")
}

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

/// Streams a model into `partial`: Concat's own mirror first, the upstream
/// it was filled from second, and the finished file checked against the
/// digest its table carries before the caller is told it arrived.
///
/// `file` is what the model is called on the mirror. Two tries and not one
/// because a mirror that cannot be reached - a proxy that blocks our host,
/// a release still being published - should cost a retry rather than a
/// feature; see [`concat_host::models`]. A file that arrives and hashes
/// wrong is refused outright rather than retried, since the second try
/// would only hide which source served it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fetch_model(
    file: &str,
    upstream: &str,
    partial: &std::path::Path,
    id: &str,
    estimate: u64,
    sha256: &str,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &mut dyn FnMut(DownloadProgress),
) -> Result<(u64, u64), String> {
    use std::sync::atomic::Ordering;

    let mut last = String::new();
    for url in concat_host::models::sources(file, upstream) {
        match download_to(&url, partial, id, estimate, cancel, progress) {
            Ok(totals) => {
                concat_host::models::verify(partial, sha256).inspect_err(|_| {
                    let _ = std::fs::remove_file(partial);
                })?;
                return Ok(totals);
            }
            // The partial stays for the next source, or the next time:
            // whichever answers takes up where this one stopped.
            Err(error) => {
                if cancel.load(Ordering::Relaxed) {
                    return Err(error);
                }
                last = error;
            }
        }
    }
    Err(last)
}

/// Streams `url` into `partial` through the shared downloader - taking
/// up from whatever of `partial` is already there - reporting every
/// couple of megabytes and stopping when `cancel` is set.
fn download_to(
    url: &str,
    partial: &std::path::Path,
    id: &str,
    estimate: u64,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &mut dyn FnMut(DownloadProgress),
) -> Result<(u64, u64), String> {
    concat_host::models::download(
        url,
        partial,
        estimate,
        cancel,
        "download cancelled",
        &mut |received, total| {
            progress(DownloadProgress {
                id: id.to_owned(),
                received,
                total,
                unpacking: false,
                done: false,
            })
        },
    )
}
