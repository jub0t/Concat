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
    treat_to(frame, frame.width(), frame.height(), chain)
}

/// Runs `frame` through `chain` and returns the result at `width` by
/// `height`: the chain at the picture's own size, then the scale. An empty
/// chain at the same size returns a copy.
pub fn treat_to(frame: &Frame, width: u32, height: u32, chain: &str) -> Result<Frame> {
    if chain.trim().is_empty() && width == frame.width() && height == frame.height() {
        return Ok(frame.clone());
    }
    let spec = if chain.trim().is_empty() {
        format!("scale={width}:{height}:flags=bilinear,format=rgba")
    } else {
        format!("{chain},scale={width}:{height}:flags=bilinear,format=rgba")
    };
    run(frame, &spec)
}

/// Runs a source frame the way the decoder would have: `pre` in the
/// picture's own pixels - the crop - then the fit to `width` by `height`,
/// then `chain` at that size, then the guard scale that pins the size. The
/// decoder's graph without the decoder, for a picture the reader pool
/// holds untreated. Nothing to do at the same size returns a copy.
pub fn fit(
    frame: &Frame,
    pre: Option<&str>,
    width: u32,
    height: u32,
    chain: Option<&str>,
) -> Result<Frame> {
    let pre = pre.filter(|pre| !pre.trim().is_empty());
    let chain = chain.filter(|chain| !chain.trim().is_empty());
    if pre.is_none() && chain.is_none() && width == frame.width() && height == frame.height() {
        return Ok(frame.clone());
    }
    let mut parts: Vec<String> = Vec::new();
    if let Some(pre) = pre {
        parts.push(pre.to_owned());
    }
    parts.push(format!("scale={width}:{height}:flags=bilinear"));
    if let Some(chain) = chain {
        parts.push(chain.to_owned());
        parts.push(format!("scale={width}:{height}:flags=bilinear"));
    }
    parts.push("format=rgba".to_owned());
    run(frame, &parts.join(","))
}

/// How many graphs a thread keeps; see [`GRAPHS`].
const GRAPHS_KEPT: usize = 8;

/// A kept graph: what it was built for, and the graph.
type Kept = ((String, u32, u32), filter::Graph);

thread_local! {
    /// Built graphs by their spec and input size, this thread's own: a
    /// graph is a parse and a validation, which the monitor path paid for
    /// every treated frame (audit 2026-09-23, #16). A worker treats one or
    /// two clips at a time, so a few are plenty; the oldest goes when the
    /// cap is reached. A graph is taken out while in use, so a re-entrant
    /// call builds its own.
    static GRAPHS: std::cell::RefCell<Vec<Kept>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// One RGBA picture in, `spec` - a whole filtergraph, ending in the size
/// and format the caller wants out - and one RGBA picture out.
fn run(frame: &Frame, spec: &str) -> Result<Frame> {
    ffi::init();
    let key = (spec.to_owned(), frame.width(), frame.height());
    let cached = GRAPHS.with(|graphs| {
        let mut graphs = graphs.borrow_mut();
        graphs
            .iter()
            .position(|(known, _)| *known == key)
            .map(|index| graphs.remove(index).1)
    });
    let mut graph = match cached {
        Some(graph) => graph,
        None => build(spec, frame.width(), frame.height())?,
    };
    let result = pass(&mut graph, frame);
    if result.is_ok() {
        GRAPHS.with(|graphs| {
            let mut graphs = graphs.borrow_mut();
            if graphs.len() >= GRAPHS_KEPT {
                graphs.remove(0);
            }
            graphs.push((key, graph));
        });
    }
    result
}

/// The graph for `spec` over RGBA pictures `width` by `height`.
fn build(spec: &str, width: u32, height: u32) -> Result<filter::Graph> {
    // The graph has no file behind it; errors name the layer instead.
    let path = Path::new("layer");
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
    graph
        .output("in", 0)
        .and_then(|parser| parser.input("out", 0))
        .and_then(|parser| parser.parse(spec))
        .map_err(|error| ffi::fail("filter graph", path, error))?;
    graph
        .validate()
        .map_err(|error| ffi::fail("filter graph", path, error))?;
    Ok(graph)
}

/// One picture through `graph`: in at `in`, out at `out`.
fn pass(graph: &mut filter::Graph, frame: &Frame) -> Result<Frame> {
    let path = Path::new("layer");
    let (source_w, source_h) = (frame.width(), frame.height());

    // The picture, as a padded FFmpeg frame.
    let mut source = Video::new(Pixel::RGBA, source_w, source_h);
    {
        let row = source_w as usize * 4;
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

    /// A fit crops in the source's pixels, scales, and runs the chain at
    /// the output size: the left half of a picture red on the left and
    /// blue on the right, fitted to four across, is all red - and negated,
    /// all cyan.
    #[test]
    fn a_fit_crops_then_scales_then_treats() {
        let mut frame = Frame::black(8, 4);
        for (index, pixel) in frame.pixels_mut().chunks_exact_mut(4).enumerate() {
            let x = index % 8;
            pixel.copy_from_slice(if x < 4 {
                &[255, 0, 0, 255]
            } else {
                &[0, 0, 255, 255]
            });
        }
        let crop = "crop=w=iw/2:h=ih:x=0:y=0";
        let left = fit(&frame, Some(crop), 4, 4, None).expect("fits");
        assert_eq!((left.width(), left.height()), (4, 4));
        assert_eq!(
            left.pixel(3, 1),
            Some([255, 0, 0, 255]),
            "the left half only"
        );
        let negated = fit(&frame, Some(crop), 4, 4, Some("negate")).expect("fits and treats");
        assert_eq!(negated.pixel(3, 1), Some([0, 255, 255, 255]));
        let same = fit(&frame, None, 8, 4, None).expect("copies");
        assert_eq!(same.pixels(), frame.pixels());
    }
}
