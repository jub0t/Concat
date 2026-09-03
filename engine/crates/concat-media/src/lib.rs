// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Media IO: reading frames and samples out of files and writing them back.
//!
//! This is the only crate that knows FFmpeg exists, and it links it -
//! libavformat, libavcodec, libavfilter, libswscale and libswresample -
//! rather than spawning the `ffmpeg` binary. Nothing here needs a program on
//! `PATH`; a build needs the FFmpeg development libraries, and the binary
//! carries the rest.
//!
//! Everything codec-shaped hides behind two traits, [`FrameSource`] and
//! [`FrameSink`]. A GPU-decode backend would implement those two and nothing
//! else in the workspace changes.

pub mod audio;
pub mod decode;
pub mod encode;
pub mod error;
mod ffi;
pub mod peaks;
pub mod pool;
pub mod probe;
pub mod samples;
pub mod treat;

pub use decode::{DecodeOptions, Decoder, FrameSource, SeekableSource};
pub use encode::{EncodeOptions, Encoder, FrameSink, jpeg};
pub use error::{Error, Result};
pub use ffi::{init, linked_version};
pub use peaks::Peaks;
pub use pool::{FrameCache, ReaderPool};
pub use probe::{AudioStream, MediaInfo, VideoStream, probe};
pub use samples::{AudioDecoder, AudioOptions, SampleFormat};
pub use treat::treat;
