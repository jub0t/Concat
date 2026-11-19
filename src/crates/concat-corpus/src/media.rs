// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The media a case is built from, made fresh for every run: pictures that
//! carry their own [`Code`] and enough structure that a wrong one is not
//! mistaken for a right one.
//!
//! Only formats every OS writes the same way are made. Moving pictures are
//! ProRes 4444, which FFmpeg's own encoder writes on every machine, and
//! stills are PNG; never H.264, whose encoder is a different one on each
//! OS (and a different file on each), so a case's input would not be the
//! same input everywhere.

use std::path::Path;

use concat_core::frame::Frame;
use concat_core::time::FrameRate;
use concat_media::{EncodeOptions, Encoder, FrameSink, RateMode, VideoCodec};

use crate::code::{self, Code};

/// The colour of a picture's patch during source second `second`: a red
/// step of forty a second over a fixed green and blue, far from the grey
/// of the code and from the gradient beside it.
pub fn colour_of_second(second: u32) -> [u8; 4] {
    [(30 + 40 * (second % 6)) as u8, 60, 200, 255]
}

/// The side of the square patch at the bottom right of every picture.
const PATCH: u32 = 48;

/// One picture: a gradient body, bright everywhere so it is never taken
/// for the black of a bar or a gap, the patch for source second `second`
/// at the bottom right, and the code at the top left.
pub fn picture(width: u32, height: u32, code: Code, second: u32) -> Frame {
    let mut frame = Frame::black(width, height);
    let stride = width as usize * 4;
    let patch = colour_of_second(second);
    let pixels = frame.pixels_mut();
    for y in 0..height {
        for x in 0..width {
            let in_patch = x + PATCH >= width && y + PATCH >= height;
            let rgba = if in_patch {
                patch
            } else {
                [
                    (96 + x * 128 / width) as u8,
                    (96 + y * 128 / height) as u8,
                    160,
                    255,
                ]
            };
            let at = y as usize * stride + x as usize * 4;
            pixels[at..at + 4].copy_from_slice(&rgba);
        }
    }
    code::write_code(&mut frame, 0, 0, code);
    frame
}

/// A silent clip of `frames` pictures at `rate`, `width` × `height`, in
/// ProRes 4444: frame `n` carries media `tag`, frame `n`.
pub fn clock(
    path: &Path,
    width: u32,
    height: u32,
    rate: FrameRate,
    frames: u32,
    tag: u8,
) -> Result<(), String> {
    let options = EncodeOptions {
        codec: VideoCodec::ProRes4444,
        preset: "medium".to_owned(),
        crf: 0,
        rate_mode: RateMode::Vbr,
        bitrate_kbps: 0,
        ten_bit: true,
        color_range: concat_media::ColorRange::Limited,
        hardware: false,
        threads: 0,
        software_only: true,
    };
    let fail =
        |what: &str, error: &dyn std::fmt::Display| format!("{what} {}: {error}", path.display());
    let mut encoder = Encoder::create(path, width, height, rate, &options)
        .map_err(|error| fail("could not start", &error))?;
    let fps = rate.fps().as_f64();
    for index in 0..frames {
        let second = (f64::from(index) / fps).floor() as u32;
        let frame = picture(width, height, Code { tag, index }, second);
        encoder
            .write_frame(&frame)
            .map_err(|error| fail("could not write a frame of", &error))?;
    }
    encoder
        .finish()
        .map_err(|error| fail("could not finish", &error))
}

/// A still, `width` × `height`, as an opaque RGB PNG carrying media `tag`,
/// frame nought.
pub fn still(path: &Path, width: u32, height: u32, tag: u8) -> Result<(), String> {
    let frame = picture(width, height, Code { tag, index: 0 }, 0);
    let rgb: Vec<u8> = frame
        .pixels()
        .chunks_exact(4)
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect();
    write_png(path, width, height, png::ColorType::Rgb, &rgb)
}

/// Writes `frame` as an RGBA PNG: a frame a report shows.
pub fn save_frame(path: &Path, frame: &Frame) -> Result<(), String> {
    let mut rgba = Vec::with_capacity(frame.pixels().len());
    for y in 0..frame.height() {
        for x in 0..frame.width() {
            rgba.extend_from_slice(&frame.pixel(x, y).unwrap_or([0; 4]));
        }
    }
    write_png(
        path,
        frame.width(),
        frame.height(),
        png::ColorType::Rgba,
        &rgba,
    )
}

fn write_png(
    path: &Path,
    width: u32,
    height: u32,
    colour: png::ColorType,
    data: &[u8],
) -> Result<(), String> {
    let fail =
        |error: &dyn std::fmt::Display| format!("could not write {}: {error}", path.display());
    let file = std::fs::File::create(path).map_err(|error| fail(&error))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(colour);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|error| fail(&error))?;
    writer
        .write_image_data(data)
        .map_err(|error| fail(&error))?;
    writer.finish().map_err(|error| fail(&error))
}

/// Decodes a PNG to an 8-bit RGBA frame: a preview the API handed back.
pub fn read_png(bytes: &[u8]) -> Result<Frame, String> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("the preview is not a PNG: {error}"))?;
    let mut data = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut data)
        .map_err(|error| format!("the preview does not decode: {error}"))?;
    data.truncate(info.buffer_size());
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return Err(format!(
            "the preview is {:?} at {:?}, not 8-bit RGBA",
            info.color_type, info.bit_depth
        ));
    }
    Frame::from_rgba(info.width, info.height, data)
        .ok_or_else(|| "the preview's size does not match its pixels".to_owned())
}
