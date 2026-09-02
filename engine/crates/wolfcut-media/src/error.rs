// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! What can go wrong when talking to FFmpeg.

use std::path::PathBuf;
use std::process::ExitStatus;

/// Shorthand for results in this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// A media IO failure.
///
/// The messages name the program and the file, because the first question you
/// will ask six months from now is "which file, and what did FFmpeg say".
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The `ffmpeg` or `ffprobe` binary could not be started at all.
    #[error("could not run `{program}` - is FFmpeg installed and on PATH?")]
    Spawn {
        /// Which binary we tried to start.
        program: &'static str,
        /// The underlying OS error.
        #[source]
        source: std::io::Error,
    },

    /// The child process ran and exited unhappily.
    #[error("`{program}` exited with {status} while handling {path}: {stderr}")]
    Exited {
        /// Which binary failed.
        program: &'static str,
        /// The file it was working on.
        path: PathBuf,
        /// Its exit status.
        status: ExitStatus,
        /// The tail of what it printed - the actual reason, when there is one.
        stderr: String,
    },

    /// An IO error moving bytes over the pipe.
    #[error("io error talking to `{program}`")]
    Io {
        /// Which binary we were talking to.
        program: &'static str,
        /// The underlying OS error.
        #[source]
        source: std::io::Error,
    },

    /// A user-supplied filter chain would break out of its slot in a
    /// filtergraph.
    #[error("invalid filter chain {chain:?}: {detail}")]
    InvalidFilterChain {
        /// The chain as supplied.
        chain: String,
        /// What made it unacceptable.
        detail: String,
    },

    /// `ffprobe` returned something we could not make sense of.
    #[error("could not understand ffprobe output for {path}: {detail}")]
    Probe {
        /// The file being probed.
        path: PathBuf,
        /// What specifically was missing or malformed.
        detail: String,
    },

    /// The file has no video stream, but we were asked to decode pictures.
    #[error("{path} has no video stream")]
    NoVideoStream {
        /// The file in question.
        path: PathBuf,
    },

    /// The decoder stopped mid-frame, which means FFmpeg died partway through.
    #[error("{path} produced a partial frame ({got} of {want} bytes)")]
    PartialFrame {
        /// The file being decoded.
        path: PathBuf,
        /// How many bytes arrived.
        got: usize,
        /// How many bytes a frame needs.
        want: usize,
    },

    /// A libav* call failed. Only produced by the linked backend.
    #[error("{operation} failed for {path}: {detail}")]
    Ffi {
        /// The libav function that failed.
        operation: &'static str,
        /// The file being worked on.
        path: PathBuf,
        /// FFmpeg's own description of the error code.
        detail: String,
    },

    /// A frame handed to an encoder was not the size the encoder was opened for.
    #[error("expected {want_width}x{want_height} frames, got {got_width}x{got_height}")]
    FrameSizeMismatch {
        /// Width the encoder was opened with.
        want_width: u32,
        /// Height the encoder was opened with.
        want_height: u32,
        /// Width of the offending frame.
        got_width: u32,
        /// Height of the offending frame.
        got_height: u32,
    },
}
