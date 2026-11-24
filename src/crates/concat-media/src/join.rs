// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Joining a video encoded in parts back into one file, without encoding
//! it again.
//!
//! An export written in parts (see `concat-export`'s `parts`) has each
//! part start on a keyframe and its timestamps start at its own zero. The
//! parts are copied one after another into a file of the same container,
//! each shifted by where it starts, so the file plays as if it had been
//! encoded in one go: same picture, same packets, and the header the
//! encoder wrote, which every part has to share.

use std::path::{Path, PathBuf};

use concat_core::time::Rational;
use ffmpeg_the_third as ffmpeg;
use ffmpeg_the_third::codec::encoder;

use crate::error::{Error, Result};
use crate::ffi;

/// One part to join: its file, and where it starts in the whole, in
/// seconds.
#[derive(Clone, Debug)]
pub struct Part {
    /// The part's file, in the container the whole is written in.
    pub path: PathBuf,
    /// Where its first frame falls in the whole.
    pub start: Rational,
}

/// What the parts must agree on to be copied into one stream: the codec,
/// the picture's size and format, and the encoder's header (an H.264
/// file's SPS and PPS). Parts that disagree cannot be one file; a reader
/// would decode the later ones against the first one's header.
#[derive(PartialEq, Eq, Debug)]
struct Shape {
    codec: ffmpeg::codec::Id,
    width: i32,
    height: i32,
    format: i32,
    header: Vec<u8>,
}

fn shape_of(parameters: &ffmpeg::codec::ParametersRef<'_>) -> Shape {
    // SAFETY: `parameters` wraps a live AVCodecParameters owned by its
    // stream; its extradata is `extradata_size` bytes, or null with none.
    let (width, height, format, header) = unsafe {
        let raw = &*parameters.as_ptr();
        let header = if raw.extradata.is_null() || raw.extradata_size <= 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(raw.extradata, raw.extradata_size as usize).to_vec()
        };
        (raw.width, raw.height, raw.format, header)
    };
    Shape {
        codec: parameters.id(),
        width,
        height,
        format,
        header,
    }
}

/// Copies the video stream of each of `parts`, in order, into `output`,
/// each shifted to start at its `start`. The parts must share one shape
/// (see [`Shape`]); a part that does not is refused before anything is
/// written past the header, and the error says which.
pub fn join_video(parts: &[Part], output: &Path) -> Result<()> {
    ffi::init();
    if parts.is_empty() {
        return Err(Error::Ffi {
            operation: "join",
            path: output.to_path_buf(),
            detail: "there are no parts to join".into(),
        });
    }
    let mut inputs = Vec::with_capacity(parts.len());
    let mut shape = None;
    for part in parts {
        let input = ffmpeg::format::input(&part.path)
            .map_err(|error| ffi::fail("open", &part.path, error))?;
        let stream = input
            .streams()
            .best(ffmpeg::media::Type::Video)
            .ok_or_else(|| Error::NoVideoStream {
                path: part.path.clone(),
            })?;
        let this = shape_of(&stream.parameters());
        match &shape {
            None => shape = Some(this),
            Some(wanted) if *wanted != this => {
                return Err(Error::Ffi {
                    operation: "join",
                    path: part.path.clone(),
                    detail: format!(
                        "this part was encoded differently from the first ({:?} {}x{}, {} header bytes, against {:?} {}x{}, {})",
                        this.codec,
                        this.width,
                        this.height,
                        this.header.len(),
                        wanted.codec,
                        wanted.width,
                        wanted.height,
                        wanted.header.len()
                    ),
                });
            }
            Some(_) => {}
        }
        let (index, time_base) = (stream.index(), stream.time_base());
        inputs.push((input, index, time_base));
    }

    let mut out =
        ffmpeg::format::output(output).map_err(|error| ffi::fail("create", output, error))?;
    {
        let (input, index, time_base) = &inputs[0];
        let stream = input.stream(*index).expect("the stream found above");
        let mut added = out
            .add_stream(encoder::find(ffmpeg::codec::Id::None))
            .map_err(|error| ffi::fail("add stream", output, error))?;
        // The first part's parameters, its tag with them: an HEVC
        // picture stays hvc1, which Apple's players need.
        added.set_parameters(stream.parameters());
        added.set_time_base(*time_base);
        if let Some(mode) = stream.metadata().get("alpha_mode") {
            added.metadata_mut().set("alpha_mode", mode);
        }
    }
    out.write_header()
        .map_err(|error| ffi::fail("write header", output, error))?;
    let to = out
        .stream(0)
        .map(|stream| stream.time_base())
        .unwrap_or(inputs[0].2);

    // The last decode time written, in the output's ticks: a part's first
    // packets may decode before its start (a B-frame encoder's lead), and
    // a muxer refuses a decode time that does not move forward.
    let mut last_dts: Option<i64> = None;
    for ((mut input, index, from), part) in inputs.into_iter().zip(parts) {
        let shift = ticks_of(part.start, to);
        for item in input.packets() {
            let (stream, mut packet) =
                item.map_err(|error| ffi::fail("read", &part.path, error))?;
            if stream.index() != index {
                continue;
            }
            packet.rescale_ts(from, to);
            if let Some(pts) = packet.pts() {
                packet.set_pts(Some(pts + shift));
            }
            if let Some(dts) = packet.dts() {
                let mut dts = dts + shift;
                if let Some(last) = last_dts
                    && dts <= last
                {
                    dts = last + 1;
                }
                if let Some(pts) = packet.pts() {
                    dts = dts.min(pts);
                }
                packet.set_dts(Some(dts));
                last_dts = Some(dts);
            }
            packet.set_stream(0);
            packet.set_position(-1);
            packet
                .write_interleaved(&mut out)
                .map_err(|error| ffi::fail("write packet", output, error))?;
        }
    }
    out.write_trailer()
        .map_err(|error| ffi::fail("write trailer", output, error))?;
    Ok(())
}

/// `seconds` in ticks of `base`, to the nearest tick.
fn ticks_of(seconds: Rational, base: ffmpeg::Rational) -> i64 {
    let ticks = seconds * Rational::new(i64::from(base.denominator()), i64::from(base.numerator()));
    (ticks + Rational::new(1, 2)).floor()
}
