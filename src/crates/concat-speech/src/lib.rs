// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Speech in and out, entirely on this machine.
//!
//! - [`transcribe`] - whisper.cpp, in-process, turns a clip's audio into
//!   timed caption segments.
//! - [`tts`] - Chatterbox on the desktop, and the phone's own voice on
//!   Android, turn typed narration into a WAV in the project folder.
//!
//! Both download their models on demand into the app's data directory and
//! never bundle them. Both are one-at-a-time: a [`concat_host::SingleFlight`]
//! refuses a second concurrent run rather than letting two share a cancel
//! flag.
//!
//! A separate crate from `concat-host` so the heavy native dependencies
//! (whisper.cpp, and ONNX Runtime for Chatterbox) stay out of everything
//! that does not speak.

pub mod apart;
#[cfg(feature = "beats")]
pub mod beats;
#[cfg(feature = "chatterbox")]
pub mod chatterbox;
#[cfg(feature = "isolate")]
pub mod isolate;
pub mod transcribe;
pub mod tts;
#[cfg(feature = "isolate")]
pub mod vocals;

use concat_core::Halt;
pub use transcribe::Transcriber;
pub use tts::Speech;

/// How far a transcription or a read has got, as it reports it: the stage
/// it is in, and in the work itself where it is and where the step under
/// way will leave it. A job is mostly long waits between reports - whisper
/// reports once per thirty seconds of sound, Chatterbox every few words - and
/// `next` is what lets a bar move through the wait without promising more
/// than the job has done.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum JobStage {
    /// Reading the sound in: a clip's, or the recording a voice is read
    /// from.
    Reading,
    /// Loading the model. Not reported when it is still loaded from the
    /// run before.
    Loading,
    /// The work itself.
    Working {
        /// How much of it is finished, `0..=1`.
        done: f32,
        /// Where the step under way ends, `done..=1`.
        next: f32,
    },
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

/// Streams a model into `partial` through the host's downloader:
/// Concat's own mirror first, the upstream it was filled from after it,
/// or the copy the person chose themselves, and the finished file checked
/// against the digest its table carries before the caller is told it
/// arrived. See [`concat_host::models::fetch`], which also says - when
/// every source fails - which hosts were tried and why they likely failed.
///
/// `file` is what the model is called on the mirror. Setting `cancel`
/// ends it with [`Halt::Cancelled`].
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
) -> Result<(u64, u64), Halt> {
    concat_host::models::fetch(
        file,
        upstream,
        partial,
        estimate,
        sha256,
        cancel,
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
