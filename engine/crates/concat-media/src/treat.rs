// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! One picture through a filter chain, in memory.
//!
//! The decoder runs every clip's own chain as the pixels come out of the
//! codec. A *layer*, meaning a look or an effect placed over a span of the
//! timeline, has no pixels of its own: it treats whatever has been
//! composited beneath it. That picture exists only in memory, so this is
//! the decoder's filtergraph without the decoder: an RGBA buffer in, the
//! chain, an RGBA buffer out. A guard scale pins the size, as the decoder's
//! does, so a chain that resizes cannot change the frame under the
//! compositor.

use std::path::Path;

use concat_core::frame::Frame;
use ffmpeg_the_third as ffmpeg;
use ffmpeg_the_third::filter;
use ffmpeg_the_third::format::Pixel;
use ffmpeg_the_third::util::frame::video::Video;

use crate::error::{Error, Result};
use crate::ffi;

/// Runs `frame` through `chain` and returns the result at the same size.
/// An empty chain returns a copy.
pub fn treat(frame: &Frame, chain: &str) -> Result<Frame> {
    if chain.trim().is_empty() {
        return Ok(frame.clone());
    }
    ffi::init();
    // The graph has no file behind it; errors name the layer instead.
    let path = Path::new("layer");
    let (width, height) = (frame.width(), frame.height());

    let mut graph = filter::Graph::new();
    let args = format!(
        "video_size={width}x{height}:pix_fmt={}:time_base=1/1000:pixel_aspect=1/1",
        Into::<ffmpeg::sys::AVPixelFormat>::into(Pixel::RGBA).0
    );
    let missing = |name: &str| Error::Missing {
        what: "filter",
        name: name.to_owned(),
    };
    graph
        .add(
            &filter::find("buffer").ok_or_else(|| missing("buffer"))?,
            "in",
            &args,
        )
        .map_err(|error| ffi::fail("buffer source", path, error))?;
    graph
        .add(
            &filter::find("buffersink").ok_or_else(|| missing("buffersink"))?,
            "out",
            "",
        )
        .map_err(|error| ffi::fail("buffer sink", path, error))?;
    let spec = format!("{chain},scale={width}:{height}:flags=bilinear,format=rgba");
    graph
        .output("in", 0)
        .and_then(|parser| parser.input("out", 0))
        .and_then(|parser| parser.parse(&spec))
        .map_err(|error| ffi::fail("filter graph", path, error))?;
    graph
        .validate()
        .map_err(|error| ffi::fail("filter graph", path, error))?;

    // The picture, as a padded FFmpeg frame.
    let mut source = Video::new(Pixel::RGBA, width, height);
    {
        let row = width as usize * 4;
        let stride = source.stride(0);
        let data = source.data_mut(0);
        for (y, line) in frame.pixels().chunks_exact(row).enumerate() {
            data[y * stride..y * stride + row].copy_from_slice(line);
        }
    }
    source.set_pts(Some(0));
    {
        let mut context = graph.get("in").expect("the graph has an input");
        context
            .source()
            .add(&source)
            .map_err(|error| ffi::fail("filter", path, error))?;
    }
    let mut filtered = Video::empty();
    {
        let mut context = graph.get("out").expect("the graph has an output");
        context
            .sink()
            .frame(&mut filtered)
            .map_err(|error| ffi::fail("filter output", path, error))?;
    }

    // Back to a packed buffer; only the row padding differs.
    let out_width = filtered.width();
    let out_height = filtered.height();
    let row = out_width as usize * 4;
    let stride = filtered.stride(0);
    let data = filtered.data(0);
    let mut pixels = Vec::with_capacity(row * out_height as usize);
    for y in 0..out_height as usize {
        pixels.extend_from_slice(&data[y * stride..y * stride + row]);
    }
    Frame::from_rgba(out_width, out_height, pixels).ok_or_else(|| Error::Probe {
        path: path.to_path_buf(),
        detail: "the filtergraph produced a frame of the wrong size".to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A solid red picture through `negate` comes back cyan, the same size.
    #[test]
    fn a_chain_changes_the_pixels_and_keeps_the_size() {
        let mut frame = Frame::black(8, 4);
        for pixel in frame.pixels_mut().chunks_exact_mut(4) {
            pixel.copy_from_slice(&[255, 0, 0, 255]);
        }
        let out = treat(&frame, "negate").expect("negate is a filter FFmpeg has");
        assert_eq!((out.width(), out.height()), (8, 4));
        assert_eq!(&out.pixels()[..4], &[0, 255, 255, 255]);
    }

    /// An empty chain is a copy, and a chain that resizes is pinned back.
    #[test]
    fn empty_is_a_copy_and_the_size_is_guarded() {
        let frame = Frame::black(6, 6);
        let same = treat(&frame, "").expect("copies");
        assert_eq!(same.pixels(), frame.pixels());
        let pinned = treat(&frame, "scale=3:3").expect("scales then pins");
        assert_eq!((pinned.width(), pinned.height()), (6, 6));
    }
}
