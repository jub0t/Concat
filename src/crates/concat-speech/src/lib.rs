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

/// Whether the voices ask for the machine's own accelerator: CoreML, the
/// Mac's GPU and Neural Engine. Not a setting. Measured on an M5 against
/// the CPU, a warm read took 10.3 s against 14.8 for Kokoro (and came out
/// sample for sample the same), 5.6 against 6.5 for Pocket and 48.6
/// against 60.6 for Chatterbox. The one cost is Chatterbox's load, about
/// eleven seconds longer while CoreML compiles its networks, paid once per
/// run since the engine stays loaded. A model that will not load for CoreML
/// at all loads for the CPU instead (tts.rs).
pub const fn accelerated() -> bool {
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
