// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Writing RGBA frames back out to a file.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Stdio};

use concat_core::frame::Frame;
use concat_core::time::FrameRate;

use crate::error::{Error, Result};
use crate::process::{StderrTail, base_command};

/// Name used in error messages. The binary actually run comes from
/// [`crate::binaries::ffmpeg`], which may be a bundled copy.
const FFMPEG: &str = "ffmpeg";

/// Anything that accepts finished frames.
///
/// The mirror of [`FrameSource`](crate::decode::FrameSource): render code
/// writes to this trait, so an export target, a preview window and a test spy
/// are interchangeable.
pub trait FrameSink {
    /// Accepts one frame. Frames must all be the size the sink was opened with.
    fn write_frame(&mut self, frame: &Frame) -> Result<()>;

    /// Flushes and closes. Always call this - a dropped sink produces a
    /// truncated file, because the encoder never got to write its trailer.
    fn finish(&mut self) -> Result<()>;
}

/// Encoder settings.
#[derive(Clone, Debug)]
pub struct EncodeOptions {
    /// Video codec, as FFmpeg names it.
    pub codec: String,
    /// x264-style speed/size tradeoff.
    pub preset: String,
    /// Constant rate factor. Lower is better quality and a bigger file.
    pub crf: u8,
    /// Output pixel format. `yuv420p` is what players actually accept.
    pub pixel_format: String,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            codec: "libx264".to_owned(),
            preset: "medium".to_owned(),
            crf: 18,
            pixel_format: "yuv420p".to_owned(),
        }
    }
}

/// Encodes by piping raw RGBA into an `ffmpeg` child process.
pub struct FfmpegEncoder {
    path: PathBuf,
    child: Child,
    /// Taken in `finish` - closing the pipe is what tells FFmpeg to stop.
    stdin: Option<ChildStdin>,
    stderr: StderrTail,
    width: u32,
    height: u32,
    written: u64,
}

impl FfmpegEncoder {
    /// Opens `path` for writing, overwriting anything already there.
    pub fn create(
        path: impl AsRef<Path>,
        width: u32,
        height: u32,
        frame_rate: FrameRate,
        options: &EncodeOptions,
    ) -> Result<Self> {
        let path = path.as_ref();
        let fps = frame_rate.fps();

        let mut command = base_command(crate::binaries::ffmpeg());
        command
            .arg("-y")
            // Input: what is coming down the pipe.
            .args(["-f", "rawvideo", "-pix_fmt", "rgba"])
            .args(["-s", &format!("{width}x{height}")])
            .args(["-r", &format!("{}/{}", fps.numerator(), fps.denominator())])
            // `pipe:0`, not `-`: since FFmpeg 6 the bare dash resolves to the
            // `fd:` protocol, which a trimmed build (like the one we bundle)
            // may not include. The pipe protocol is what we actually mean.
            .args(["-i", "pipe:0"])
            // Output.
            .args(["-c:v", &options.codec])
            .args(["-preset", &options.preset])
            .args(["-crf", &options.crf.to_string()])
            .args(["-pix_fmt", &options.pixel_format])
            .args(["-movflags", "+faststart"])
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child =
            command.spawn().map_err(|source| Error::Spawn { program: FFMPEG, source })?;
        let stdin = child.stdin.take().expect("stdin was piped");
        let stderr = StderrTail::drain(&mut child);

        Ok(Self {
            path: path.to_path_buf(),
            child,
            stdin: Some(stdin),
            stderr,
            width,
            height,
            written: 0,
        })
    }

    /// How many frames have been accepted so far.
    pub const fn written(&self) -> u64 {
        self.written
    }
}

impl FrameSink for FfmpegEncoder {
    fn write_frame(&mut self, frame: &Frame) -> Result<()> {
        if frame.width() != self.width || frame.height() != self.height {
            // Raw video has no framing, so a wrong-sized frame would not fail
            // here - it would silently shear every frame after it.
            return Err(Error::FrameSizeMismatch {
                want_width: self.width,
                want_height: self.height,
                got_width: frame.width(),
                got_height: frame.height(),
            });
        }

        let Some(stdin) = self.stdin.as_mut() else {
            return Err(Error::Io {
                program: FFMPEG,
                source: std::io::Error::other("encoder was already finished"),
            });
        };

        if let Err(source) = stdin.write_all(frame.pixels()) {
            // A broken pipe means the encoder died; what it printed on the way
            // out is the informative error, not the EPIPE it left behind.
            if source.kind() == std::io::ErrorKind::BrokenPipe {
                drop(self.stdin.take());
                if let Ok(status) = self.child.wait()
                    && !status.success()
                {
                    return Err(Error::Exited {
                        program: FFMPEG,
                        path: self.path.clone(),
                        status,
                        stderr: self.stderr.summary(),
                    });
                }
            }
            return Err(Error::Io { program: FFMPEG, source });
        }
        self.written += 1;
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        // Closing stdin is the signal to flush and write the trailer.
        drop(self.stdin.take());

        let status = self.child.wait().map_err(|source| Error::Io { program: FFMPEG, source })?;
        if status.success() {
            Ok(())
        } else {
            Err(Error::Exited {
                program: FFMPEG,
                path: self.path.clone(),
                status,
                stderr: self.stderr.summary(),
            })
        }
    }
}

impl Drop for FfmpegEncoder {
    fn drop(&mut self) {
        if self.stdin.is_some() {
            // finish() was never called, so the output is garbage anyway.
            // Kill the child instead of blocking a drop on an encode flush.
            drop(self.stdin.take());
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_a_playable_h264_file() {
        let options = EncodeOptions::default();
        assert_eq!(options.codec, "libx264");
        assert_eq!(options.pixel_format, "yuv420p", "yuv444 will not play in browsers");
    }

    #[test]
    fn a_wrong_sized_frame_is_rejected() {
        let Ok(mut encoder) = FfmpegEncoder::create(
            std::env::temp_dir().join("concat-encode-size-test.mp4"),
            64,
            64,
            FrameRate::THIRTY,
            &EncodeOptions::default(),
        ) else {
            // No FFmpeg on this machine; nothing to assert.
            return;
        };

        let wrong = Frame::black(32, 32);
        assert!(matches!(
            encoder.write_frame(&wrong),
            Err(Error::FrameSizeMismatch { got_width: 32, .. })
        ));
    }
}
