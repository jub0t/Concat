// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! What can go wrong when talking to FFmpeg.

use std::path::PathBuf;

/// Shorthand for results in this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// A media IO failure.
///
/// The messages name the operation and the file, because the first question
/// you will ask six months from now is "which file, and what did FFmpeg say".
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A libav* call failed.
    #[error("{operation} failed for {path}: {detail}")]
    Ffi {
        /// The libav function or step that failed.
        operation: &'static str,
        /// The file being worked on.
        path: PathBuf,
        /// FFmpeg's own description of the error.
        detail: String,
    },

    /// An IO error on a file this crate reads or writes itself.
    #[error("io error on {path}")]
    Io {
        /// The file.
        path: PathBuf,
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

    /// The container said something we could not make sense of.
    #[error("could not understand {path}: {detail}")]
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

    /// The file has no audio stream, but we were asked to decode sound.
    #[error("{path} has no audio stream")]
    NoAudioStream {
        /// The file in question.
        path: PathBuf,
    },

    /// A seek landed past every decodable frame.
    #[error("{path} has no frame to show at the requested time")]
    NoFrame {
        /// The file being decoded.
        path: PathBuf,
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

    /// The linked FFmpeg was built without something this operation needs -
    /// an encoder, a filter, a muxer.
    #[error("the linked FFmpeg has no {what} {name:?}")]
    Missing {
        /// "encoder", "filter", "muxer".
        what: &'static str,
        /// Its name.
        name: String,
    },
}
