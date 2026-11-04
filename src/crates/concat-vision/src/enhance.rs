// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Enhance: a picture restored and enlarged by a model.
//!
//! The model is Real-ESRGAN's compact general network: a frame in, the
//! same frame four times the size out, with its noise, blocking and
//! softness gone. It reads any size in principle, but a whole frame at
//! once is more memory than a phone has and a shape the accelerators
//! compile for badly, so the frame is cut into tiles of one fixed size,
//! each run with a margin of its neighbours around it so the seams do
//! not show, and the answers are pasted back. Everything about the tiling
//! is a pure function here, tested without a model; only [`Enhancer`]
//! needs one, and the `infer` feature that brings the runtime.
//!
//! Four times is more than a clip wants. Past 2K - 2560 by 1440, what a
//! consumer machine plays and a phone or laptop screen shows - enlarging
//! buys time and gigabytes and no picture anybody sees, so nothing is made
//! larger than [`MAX_LONG`] by [`MAX_SHORT`]. Inside that the size is the
//! person's, any scale in hundredths - see [`scale_of`] - and Auto is
//! twice, or as near as fits. The model's four-times answer is averaged
//! down to the exact size by [`area_resize`], which keeps the restoration
//! and drops only the pixels.
//!
//! The model's answer is also as strong as it is: skin smoothed to wax,
//! grain scrubbed to plastic, which is the complaint every one-button
//! enhancer earns. [`mix`] blends it back towards the original, enlarged
//! by an ordinary bicubic filter to the same size, by a strength the
//! person picks - most of the restoration, with some of the texture the
//! model took away left in.

use concat_core::Frame;
use concat_core::frame::BYTES_PER_PIXEL;

/// How much larger the model's answer is than what it read.
pub const SCALE: u32 = 4;
/// The square of source pixels one run restores.
pub const CORE: u32 = 256;
/// The margin of neighbouring pixels read around a tile and thrown away
/// after, so the tile's edge is restored knowing what is past it.
pub const PAD: u32 = 16;
/// The square the model actually reads: the tile and its margin.
pub const WINDOW: u32 = CORE + 2 * PAD;
/// The longest side Enhance will make: 2K's. A picture already past it is
/// restored at its own size rather than enlarged.
pub const MAX_LONG: u32 = 2560;
/// The shorter side's limit, 2K's 1440, so an upright phone video is held
/// to 1440 across as a landscape one is held to 1440 high.
pub const MAX_SHORT: u32 = 1440;
/// The smallest step a scale is asked in, so a copy made at one scale is
/// found again rather than made twice for a slider's rounding.
pub const SCALE_STEP: f32 = 0.05;

/// One tile: a rectangle of source pixels restored in one run. Every tile
/// but the last in a row or column is [`CORE`] square.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    /// Left edge, in source pixels.
    pub x: u32,
    /// Top edge, in source pixels.
    pub y: u32,
    /// Width, in source pixels: `CORE`, or what is left of the row.
    pub width: u32,
    /// Height, in source pixels: `CORE`, or what is left of the column.
    pub height: u32,
}

/// The tiles of a `width` by `height` picture, row by row: they cover it
/// exactly, and no pixel is in two. Empty for an empty picture.
pub fn tiles(width: u32, height: u32) -> Vec<Tile> {
    let mut tiles = Vec::new();
    let mut y = 0;
    while y < height {
        let mut x = 0;
        while x < width {
            tiles.push(Tile {
                x,
                y,
                width: CORE.min(width - x),
                height: CORE.min(height - y),
            });
            x += CORE;
        }
        y += CORE;
    }
    tiles
}

/// `scale` on the [`SCALE_STEP`] grid, rounded down so it never steps past
/// the limit it was clamped to.
fn on_grid(scale: f32) -> f32 {
    ((scale / SCALE_STEP) + 1e-3).floor() * SCALE_STEP
}

/// The most a `width` by `height` picture may be enlarged: as far as 2K
/// on both sides allows, exactly - 1080p's is 1440p's four thirds, off
/// the grid - never past the model's own four times, and never under
/// once: a picture already past 2K is restored at its own size.
pub fn max_scale(width: u32, height: u32) -> f32 {
    let (long, short) = (width.max(height), width.min(height));
    if long == 0 || short == 0 {
        return SCALE as f32;
    }
    let most = (MAX_LONG as f32 / long as f32)
        .min(MAX_SHORT as f32 / short as f32)
        .min(SCALE as f32);
    most.max(1.0)
}

/// Auto: twice, or as much as 2K leaves when that is less.
pub fn auto_scale(width: u32, height: u32) -> f32 {
    max_scale(width, height).min(2.0)
}

/// How many times larger a `width` by `height` picture comes out when the
/// person asked for `asked`: zero or less is Auto, [`auto_scale`];
/// otherwise the ask on the [`SCALE_STEP`] grid, held between once and
/// [`max_scale`]. The ask is a setting that outlives the clip it was made
/// on, so 4x picked on a small clip and carried to a 4K one comes out
/// once, not at 16K.
pub fn scale_of(asked: f32, width: u32, height: u32) -> f32 {
    if !(asked > 0.0) {
        return auto_scale(width, height);
    }
    let most = max_scale(width, height);
    // The top of the slider is the limit itself, which is off the grid.
    if asked >= most - SCALE_STEP / 2.0 {
        return most;
    }
    on_grid(asked + 1e-4).clamp(1.0, most)
}

/// The size a `width` by `height` picture comes out at `scale`: each side
/// rounded and made even, which a video encoder wants, and never under 2.
pub fn size_at(width: u32, height: u32, scale: f32) -> (u32, u32) {
    let side = |length: u32| {
        let scaled = (length as f32 * scale.max(1.0)).round() as u32;
        (scaled & !1).max(2)
    };
    (side(width), side(height))
}

/// The rectangle of a `width` by `height` picture, at its centre, that
/// comes out no larger than `most_width` by `most_height` once enlarged
/// `scale` times: what a preview restores, so that it shows the model's
/// work at full size in seconds rather than the whole frame in minutes.
/// The whole picture when it is smaller than that.
pub fn centre_region(
    width: u32,
    height: u32,
    scale: f32,
    most_width: u32,
    most_height: u32,
) -> Tile {
    let scale = scale.max(1.0);
    let w = ((most_width as f32 / scale) as u32).max(1).min(width);
    let h = ((most_height as f32 / scale) as u32).max(1).min(height);
    Tile {
        x: (width - w) / 2,
        y: (height - h) / 2,
        width: w,
        height: h,
    }
}

/// The pixels of `frame` inside `region`, which must lie inside it.
pub fn cut(frame: &Frame, region: Tile) -> Frame {
    let mut out = Frame::black(region.width, region.height);
    let row = region.width as usize * BYTES_PER_PIXEL;
    let stride = frame.width() as usize * BYTES_PER_PIXEL;
    let source = frame.pixels();
    let pixels = out.pixels_mut();
    for y in 0..region.height as usize {
        let at = (region.y as usize + y) * stride + region.x as usize * BYTES_PER_PIXEL;
        pixels[y * row..(y + 1) * row].copy_from_slice(&source[at..at + row]);
    }
    out
}

/// The Catmull-Rom weights of the four source samples around a point `t`
/// of the way from the second to the third: sharp enough that an
/// enlargement is not mush, with less ringing than a longer filter, and
/// summing to one so a flat picture stays flat.
fn cubic_weights(t: f32) -> [f32; 4] {
    let (t2, t3) = (t * t, t * t * t);
    [
        0.5 * (-t3 + 2.0 * t2 - t),
        0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
        0.5 * (-3.0 * t3 + 4.0 * t2 + t),
        0.5 * (t3 - t2),
    ]
}

/// For each of `to` output samples along one side, the four samples it
/// reads of a side `from` long, the edge repeated past the end, and their
/// weights. Pixel centres line up: output sample `i` sits at source
/// position `(i + 0.5) * from / to - 0.5`.
fn taps(from: u32, to: u32) -> Vec<([usize; 4], [f32; 4])> {
    let ratio = from as f32 / to.max(1) as f32;
    let last = i64::from(from.saturating_sub(1));
    (0..to)
        .map(|i| {
            let at = (i as f32 + 0.5) * ratio - 0.5;
            let base = at.floor();
            let weights = cubic_weights(at - base);
            let base = base as i64;
            let index = |k: i64| (base + k).clamp(0, last) as usize;
            ([index(-1), index(0), index(1), index(2)], weights)
        })
        .collect()
}

/// `frame` enlarged to `width` by `height` with a bicubic filter: the
/// picture as an ordinary resize gives it, which is what Strength blends
/// the model's answer towards. Meant for enlarging; shrinking with it
/// skips pixels rather than averaging them.
pub fn resize(frame: &Frame, width: u32, height: u32) -> Frame {
    if frame.width() == width && frame.height() == height {
        return frame.clone();
    }
    let (fw, fh) = (frame.width() as usize, frame.height() as usize);
    if fw == 0 || fh == 0 || width == 0 || height == 0 {
        return Frame::black(width, height);
    }
    let (ow, oh) = (width as usize, height as usize);
    let source = frame.pixels();
    // Across first, into floats a source row at a time, then down.
    let across = taps(frame.width(), width);
    let mut wide = vec![0f32; ow * fh * BYTES_PER_PIXEL];
    for y in 0..fh {
        let row = &source[y * fw * BYTES_PER_PIXEL..(y + 1) * fw * BYTES_PER_PIXEL];
        for (x, (index, weight)) in across.iter().enumerate() {
            let at = (y * ow + x) * BYTES_PER_PIXEL;
            for c in 0..BYTES_PER_PIXEL {
                wide[at + c] = (0..4)
                    .map(|k| weight[k] * f32::from(row[index[k] * BYTES_PER_PIXEL + c]))
                    .sum();
            }
        }
    }
    let down = taps(frame.height(), height);
    let mut pixels = vec![0u8; ow * oh * BYTES_PER_PIXEL];
    for (y, (index, weight)) in down.iter().enumerate() {
        for x in 0..ow {
            let at = (y * ow + x) * BYTES_PER_PIXEL;
            for c in 0..BYTES_PER_PIXEL {
                let value: f32 = (0..4)
                    .map(|k| weight[k] * wide[(index[k] * ow + x) * BYTES_PER_PIXEL + c])
                    .sum();
                pixels[at + c] = (value + 0.5).clamp(0.0, 255.0) as u8;
            }
        }
    }
    Frame::from_rgba(width, height, pixels).expect("sides and bytes agree")
}

/// `strength` of the way from `plain` to `enhanced`, pixel by pixel: 0 is
/// `plain`, 1 is `enhanced`. The two must be one size.
pub fn blend(enhanced: &Frame, plain: &Frame, strength: f32) -> Frame {
    debug_assert!(enhanced.same_size_as(plain));
    let strength = strength.clamp(0.0, 1.0);
    let pixels = enhanced
        .pixels()
        .iter()
        .zip(plain.pixels())
        .map(|(&e, &p)| {
            let (e, p) = (f32::from(e), f32::from(p));
            (p + (e - p) * strength + 0.5).clamp(0.0, 255.0) as u8
        })
        .collect();
    Frame::from_rgba(enhanced.width(), enhanced.height(), pixels).expect("sides and bytes agree")
}

/// The model's answer `enhanced` for `original`, at `strength`: itself at
/// full strength, and otherwise blended with `original` enlarged to its
/// size by [`resize`].
pub fn mix(enhanced: Frame, original: &Frame, strength: f32) -> Frame {
    if strength >= 1.0 {
        return enhanced;
    }
    let plain = resize(original, enhanced.width(), enhanced.height());
    blend(&enhanced, &plain, strength)
}

/// The model's input for one tile: the [`WINDOW`] square around it as
/// three planes of `0..=1`, red then green then blue, row-major. Past the
/// picture's edge the edge pixel repeats, which is what a restoration
/// network expects to see there.
pub fn window_planes(frame: &Frame, tile: Tile) -> Vec<f32> {
    let (fw, fh) = (frame.width() as i64, frame.height() as i64);
    let pixels = frame.pixels();
    let side = WINDOW as usize;
    let mut planes = vec![0f32; 3 * side * side];
    for wy in 0..side {
        let sy = (tile.y as i64 + wy as i64 - PAD as i64).clamp(0, fh - 1) as usize;
        for wx in 0..side {
            let sx = (tile.x as i64 + wx as i64 - PAD as i64).clamp(0, fw - 1) as usize;
            let at = (sy * fw as usize + sx) * BYTES_PER_PIXEL;
            for c in 0..3 {
                planes[(c * side + wy) * side + wx] = f32::from(pixels[at + c]) / 255.0;
            }
        }
    }
    planes
}

/// The model's answer for one tile - three planes of `WINDOW * SCALE`
/// square, red then green then blue - pasted into `out`, which is the
/// source picture's size times [`SCALE`]. The margin is dropped: only the
/// tile's own pixels, enlarged, land. Values past `0..=1` are clamped.
pub fn paste(out: &mut Frame, tile: Tile, answer: &[f32]) -> Result<(), String> {
    let side = (WINDOW * SCALE) as usize;
    if answer.len() != 3 * side * side {
        return Err(format!(
            "enhance: the model answered {} values for a tile, not {}",
            answer.len(),
            3 * side * side
        ));
    }
    let ow = out.width() as usize;
    let pixels = out.pixels_mut();
    let (tw, th) = (
        (tile.width * SCALE) as usize,
        (tile.height * SCALE) as usize,
    );
    let (ox, oy) = ((tile.x * SCALE) as usize, (tile.y * SCALE) as usize);
    let margin = (PAD * SCALE) as usize;
    for y in 0..th {
        let ay = y + margin;
        for x in 0..tw {
            let ax = x + margin;
            let at = ((oy + y) * ow + ox + x) * BYTES_PER_PIXEL;
            for c in 0..3 {
                let value = answer[(c * side + ay) * side + ax];
                pixels[at + c] = (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
            pixels[at + 3] = 255;
        }
    }
    Ok(())
}

/// For each of `to` output samples along one side of a side `from` long,
/// the source samples its span covers and how much of each: a box filter
/// at any ratio, so a shrink averages every pixel it drops. Meant for
/// shrinking; `to` must not be more than `from`.
fn spans(from: u32, to: u32) -> Vec<Vec<(usize, f32)>> {
    let ratio = from as f64 / to.max(1) as f64;
    (0..to)
        .map(|i| {
            let (start, end) = (i as f64 * ratio, (i as f64 + 1.0) * ratio);
            let mut taps = Vec::new();
            let mut at = start.floor() as usize;
            while (at as f64) < end && at < from as usize {
                let cover = (end.min(at as f64 + 1.0) - start.max(at as f64)) as f32;
                if cover > 0.0 {
                    taps.push((at, cover / ratio as f32));
                }
                at += 1;
            }
            taps
        })
        .collect()
}

/// `frame` shrunk to `width` by `height` by averaging the area each output
/// pixel covers: the model's four-times answer brought down to any size
/// without the aliasing a sampling filter leaves. Sides larger than the
/// frame's are not its job; [`resize`] enlarges.
pub fn area_resize(frame: &Frame, width: u32, height: u32) -> Frame {
    let (fw, fh) = (frame.width(), frame.height());
    if fw == width && fh == height {
        return frame.clone();
    }
    if fw % width.max(1) == 0 && fh % height.max(1) == 0 && fw / width.max(1) == fh / height.max(1)
    {
        return downscale(frame, fw / width.max(1));
    }
    if fw == 0 || fh == 0 || width == 0 || height == 0 {
        return Frame::black(width, height);
    }
    let (ow, oh, sw) = (width as usize, height as usize, fw as usize);
    let source = frame.pixels();
    let across = spans(fw, width);
    let mut wide = vec![0f32; ow * fh as usize * BYTES_PER_PIXEL];
    for y in 0..fh as usize {
        let row = &source[y * sw * BYTES_PER_PIXEL..(y + 1) * sw * BYTES_PER_PIXEL];
        for (x, taps) in across.iter().enumerate() {
            let at = (y * ow + x) * BYTES_PER_PIXEL;
            for &(index, weight) in taps {
                for c in 0..BYTES_PER_PIXEL {
                    wide[at + c] += weight * f32::from(row[index * BYTES_PER_PIXEL + c]);
                }
            }
        }
    }
    let down = spans(fh, height);
    let mut pixels = vec![0u8; ow * oh * BYTES_PER_PIXEL];
    for (y, taps) in down.iter().enumerate() {
        for x in 0..ow {
            let mut sum = [0f32; BYTES_PER_PIXEL];
            for &(index, weight) in taps {
                let at = (index * ow + x) * BYTES_PER_PIXEL;
                for c in 0..BYTES_PER_PIXEL {
                    sum[c] += weight * wide[at + c];
                }
            }
            let at = (y * ow + x) * BYTES_PER_PIXEL;
            for c in 0..BYTES_PER_PIXEL {
                pixels[at + c] = (sum[c] + 0.5).clamp(0.0, 255.0) as u8;
            }
        }
    }
    Frame::from_rgba(width, height, pixels).expect("sides and bytes agree")
}

/// `frame` averaged down by `by` in each direction: every `by` by `by`
/// block becomes one pixel. `by` of 1 is a copy. A picture whose sides
/// are not multiples of `by` loses the remainder at the right and bottom.
pub fn downscale(frame: &Frame, by: u32) -> Frame {
    if by <= 1 {
        return frame.clone();
    }
    let (fw, fh) = (frame.width() as usize, frame.height() as usize);
    let by = by as usize;
    let (ow, oh) = (fw / by, fh / by);
    let source = frame.pixels();
    let mut pixels = vec![0u8; ow * oh * BYTES_PER_PIXEL];
    let count = (by * by) as u32;
    for y in 0..oh {
        for x in 0..ow {
            let mut sum = [0u32; 4];
            for dy in 0..by {
                for dx in 0..by {
                    let at = ((y * by + dy) * fw + x * by + dx) * BYTES_PER_PIXEL;
                    for c in 0..4 {
                        sum[c] += u32::from(source[at + c]);
                    }
                }
            }
            let at = (y * ow + x) * BYTES_PER_PIXEL;
            for c in 0..4 {
                pixels[at + c] = ((sum[c] + count / 2) / count) as u8;
            }
        }
    }
    Frame::from_rgba(ow as u32, oh as u32, pixels).expect("sides and bytes agree")
}

/// What runs the model over one window: the [`WINDOW`]-square planes
/// [`window_planes`] cuts, in; the planes [`paste`] takes, [`SCALE`] times
/// the side, out. ONNX Runtime is one, on the CPU almost everywhere; the
/// host's GPU upscaler, the same network in compute shaders of our own, is
/// the other, and the one used wherever a GPU opens.
pub trait WindowRunner: Send {
    /// The model's answer for one window.
    fn run(&mut self, planes: Vec<f32>) -> Result<Vec<f32>, String>;
}

/// The model through ONNX Runtime.
#[cfg(feature = "infer")]
struct Runtime(crate::runtime::Model);

#[cfg(feature = "infer")]
impl WindowRunner for Runtime {
    fn run(&mut self, planes: Vec<f32>) -> Result<Vec<f32>, String> {
        let side = WINDOW as usize;
        let mut answer = self.0.run(
            vec![crate::runtime::Input {
                name: "input",
                dims: vec![1, 3, side, side],
                data: planes.into(),
            }],
            &["output"],
        )?;
        Ok(std::mem::take(&mut answer[0].data))
    }
}

/// The model, loaded once and run tile by tile.
pub struct Enhancer {
    model: std::sync::Mutex<Box<dyn WindowRunner>>,
}

impl Enhancer {
    /// Loads the model from its file into ONNX Runtime; see
    /// [`crate::models::ModelId::Enhance`].
    #[cfg(feature = "infer")]
    pub fn load(file: &std::path::Path) -> Result<Enhancer, String> {
        let side = i64::from(WINDOW);
        Ok(Enhancer::with(Box::new(Runtime(
            crate::runtime::Model::from_file_fixed(file, &[("height", side), ("width", side)])?,
        ))))
    }

    /// The model run by `runner`.
    pub fn with(runner: Box<dyn WindowRunner>) -> Enhancer {
        Enhancer {
            model: std::sync::Mutex::new(runner),
        }
    }

    /// `frame` restored at [`SCALE`] times its size.
    pub fn upscale(&self, frame: &Frame) -> Result<Frame, String> {
        let whole = Tile {
            x: 0,
            y: 0,
            width: frame.width(),
            height: frame.height(),
        };
        Ok(self.upscale_region(frame, whole)?.0)
    }

    /// `region` of `frame` restored at [`SCALE`] times its size, and how
    /// long each of its tiles took the model. Each tile still reads its
    /// margin from the frame around the region, so the region comes out as
    /// it would inside the whole frame.
    pub fn upscale_region(
        &self,
        frame: &Frame,
        region: Tile,
    ) -> Result<(Frame, Vec<std::time::Duration>), String> {
        if region.width == 0 || region.height == 0 {
            return Err("enhance: an empty picture".to_owned());
        }
        let mut out = Frame::black(region.width * SCALE, region.height * SCALE);
        let mut model = self.model.lock().map_err(|_| "enhance: model poisoned")?;
        let mut times = Vec::new();
        for tile in tiles(region.width, region.height) {
            let placed = Tile {
                x: region.x + tile.x,
                y: region.y + tile.y,
                ..tile
            };
            let planes = window_planes(frame, placed);
            let started = std::time::Instant::now();
            let answer = model.run(planes)?;
            times.push(started.elapsed());
            paste(&mut out, tile, &answer)?;
        }
        Ok((out, times))
    }

    /// `frame` restored and enlarged to exactly `width` by `height`, no
    /// larger than the model's four times: its answer, averaged down to
    /// size by [`area_resize`].
    pub fn enhance(&self, frame: &Frame, width: u32, height: u32) -> Result<Frame, String> {
        let large = self.upscale(frame)?;
        Ok(area_resize(
            &large,
            width.min(large.width()),
            height.min(large.height()),
        ))
    }

    /// [`Enhancer::enhance`] for `region` of `frame` alone at `scale`, with
    /// the time each of its tiles took; see [`Enhancer::upscale_region`].
    pub fn enhance_region(
        &self,
        frame: &Frame,
        region: Tile,
        scale: f32,
    ) -> Result<(Frame, Vec<std::time::Duration>), String> {
        let scale = scale.clamp(1.0, SCALE as f32);
        let (large, times) = self.upscale_region(frame, region)?;
        let width = ((region.width as f32 * scale).round() as u32).clamp(1, large.width());
        let height = ((region.height as f32 * scale).round() as u32).clamp(1, large.height());
        Ok((area_resize(&large, width, height), times))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(width: u32, height: u32, at: impl Fn(u32, u32) -> [u8; 4]) -> Frame {
        let mut pixels = Vec::with_capacity((width * height) as usize * BYTES_PER_PIXEL);
        for y in 0..height {
            for x in 0..width {
                pixels.extend_from_slice(&at(x, y));
            }
        }
        Frame::from_rgba(width, height, pixels).expect("sized")
    }

    #[test]
    fn tiles_cover_the_picture_once_and_exactly() {
        assert!(tiles(0, 0).is_empty());
        assert!(tiles(10, 0).is_empty());
        assert_eq!(
            tiles(1, 1),
            vec![Tile {
                x: 0,
                y: 0,
                width: 1,
                height: 1
            }]
        );
        for (width, height) in [(1920, 1080), (256, 256), (257, 255), (3840, 2160), (7, 300)] {
            let all = tiles(width, height);
            let mut seen = vec![0u8; (width * height) as usize];
            for tile in &all {
                assert!(tile.width >= 1 && tile.width <= CORE, "{tile:?}");
                assert!(tile.height >= 1 && tile.height <= CORE, "{tile:?}");
                assert!(tile.x + tile.width <= width && tile.y + tile.height <= height);
                for y in tile.y..tile.y + tile.height {
                    for x in tile.x..tile.x + tile.width {
                        seen[(y * width + x) as usize] += 1;
                    }
                }
            }
            assert!(
                seen.iter().all(|&n| n == 1),
                "{width}x{height}: a pixel missed or doubled"
            );
            let full = all
                .iter()
                .filter(|t| t.width == CORE && t.height == CORE)
                .count();
            assert_eq!(
                full,
                ((width / CORE) * (height / CORE)) as usize,
                "{width}x{height}"
            );
        }
    }

    #[test]
    fn auto_is_twice_unless_that_passes_2k() {
        assert_eq!(auto_scale(1280, 720), 2.0);
        assert_eq!(auto_scale(720, 1280), 2.0);
        assert_eq!(auto_scale(640, 480), 2.0);
        // 1080p can only go to 1440p: four thirds, exactly.
        assert_eq!(size_at(1920, 1080, auto_scale(1920, 1080)), (2560, 1440));
        assert_eq!(auto_scale(2560, 1440), 1.0);
        assert_eq!(auto_scale(3840, 2160), 1.0);
        assert_eq!(auto_scale(0, 0), 2.0);
    }

    #[test]
    fn nothing_comes_out_past_2k() {
        for (w, h) in [(640, 360), (854, 480), (1280, 720), (1920, 1080), (1080, 1920), (500, 500)] {
            let most = max_scale(w, h);
            let (ow, oh) = size_at(w, h, most);
            assert!(ow.max(oh) <= MAX_LONG && ow.min(oh) <= MAX_SHORT, "{w}x{h} -> {ow}x{oh}");
            assert!(most <= 4.0);
        }
        assert_eq!(max_scale(320, 180), 4.0);
    }

    #[test]
    fn a_window_is_the_tile_with_its_neighbours_and_the_edge_repeated_past_the_picture() {
        // A 300 by 300 picture whose red is x and green is y, so any pixel
        // says where it came from.
        let frame = picture(300, 300, |x, y| [x as u8, y as u8, 7, 255]);
        let tile = tiles(300, 300)[0];
        let planes = window_planes(&frame, tile);
        let side = WINDOW as usize;
        assert_eq!(planes.len(), 3 * side * side);
        let red = |wx: usize, wy: usize| (planes[wy * side + wx] * 255.0).round() as u32;
        let green = |wx: usize, wy: usize| (planes[(side + wy) * side + wx] * 255.0).round() as u32;
        let blue =
            |wx: usize, wy: usize| (planes[(2 * side + wy) * side + wx] * 255.0).round() as u32;
        // Inside the margin, the edge pixel repeats.
        assert_eq!((red(0, 0), green(0, 0)), (0, 0));
        assert_eq!((red(5, 9), green(5, 9)), (0, 0));
        // The tile's own first pixel sits past the margin.
        assert_eq!(
            (
                red(PAD as usize, PAD as usize),
                green(PAD as usize, PAD as usize)
            ),
            (0, 0)
        );
        assert_eq!(red(PAD as usize + 10, PAD as usize), 10);
        assert_eq!(green(PAD as usize, PAD as usize + 20), 20);
        // The right margin reads the neighbouring tile, not a repeat.
        assert_eq!(red(side - 1, PAD as usize), (CORE + PAD - 1) & 0xff);
        assert_eq!(blue(3, 3), 7);

        // The last tile in the row: its right margin is past the picture.
        let last = *tiles(300, 300).last().unwrap();
        assert_eq!(
            (last.x, last.y, last.width, last.height),
            (256, 256, 44, 44)
        );
        let planes = window_planes(&frame, last);
        let red = |wx: usize, wy: usize| (planes[wy * side + wx] * 255.0).round() as u32;
        assert_eq!(red(side - 1, PAD as usize), 299 & 0xff);
    }

    #[test]
    fn a_pasted_answer_lands_on_the_tiles_own_pixels_only_and_is_clamped() {
        let mut out = Frame::black(300 * SCALE, 300 * SCALE);
        let side = (WINDOW * SCALE) as usize;
        // An answer that is 1.0 red everywhere, 2.0 (over) green, -1 blue.
        let mut answer = vec![0f32; 3 * side * side];
        answer[..side * side].fill(1.0);
        answer[side * side..2 * side * side].fill(2.0);
        answer[2 * side * side..].fill(-1.0);
        let tile = Tile {
            x: 256,
            y: 0,
            width: 44,
            height: 256,
        };
        paste(&mut out, tile, &answer).expect("fits");
        assert_eq!(out.pixel(256 * 4, 0), Some([255, 255, 0, 255]));
        assert_eq!(
            out.pixel(300 * 4 - 1, 256 * 4 - 1),
            Some([255, 255, 0, 255])
        );
        // The neighbouring tile's pixels are untouched.
        assert_eq!(out.pixel(256 * 4 - 1, 0), Some([0, 0, 0, 255]));
        assert_eq!(out.pixel(256 * 4, 256 * 4), Some([0, 0, 0, 255]));
        // A short answer is refused, not read past.
        assert!(paste(&mut out, tile, &answer[1..]).is_err());
    }

    #[test]
    fn averaging_down_takes_the_mean_and_drops_the_remainder() {
        let frame = picture(5, 3, |x, y| {
            if (x + y) % 2 == 0 {
                [200, 0, 100, 255]
            } else {
                [0, 200, 100, 255]
            }
        });
        let half = downscale(&frame, 2);
        assert_eq!((half.width(), half.height()), (2, 1));
        assert_eq!(half.pixel(0, 0), Some([100, 100, 100, 255]));
        assert_eq!(half.pixel(1, 0), Some([100, 100, 100, 255]));
        let same = downscale(&frame, 1);
        assert_eq!((same.width(), same.height()), (5, 3));
        assert_eq!(same.pixels(), frame.pixels());
        let quarter = downscale(&Frame::black(8, 8), 4);
        assert_eq!((quarter.width(), quarter.height()), (2, 2));
        let none = downscale(&Frame::black(3, 3), 4);
        assert_eq!((none.width(), none.height()), (0, 0));
    }

    #[test]
    fn an_asked_scale_is_kept_on_the_grid_and_under_the_limit() {
        assert_eq!(scale_of(0.0, 640, 360), auto_scale(640, 360));
        assert!((scale_of(1.5, 640, 360) - 1.5).abs() < 1e-4);
        assert!((scale_of(1.53, 640, 360) - 1.5).abs() < 1e-4);
        assert!((scale_of(1.55, 640, 360) - 1.55).abs() < 1e-4);
        // Past the limit falls to it; under once is once.
        assert_eq!(scale_of(4.0, 1920, 1080), max_scale(1920, 1080));
        assert_eq!(scale_of(3.0, 3840, 2160), 1.0);
        assert_eq!(scale_of(0.5, 640, 360), 1.0);
    }

    #[test]
    fn an_area_shrink_keeps_a_flat_picture_flat_and_averages_detail() {
        let flat = picture(40, 20, |_, _| [200, 100, 50, 255]);
        let small = area_resize(&flat, 15, 7);
        assert_eq!((small.width(), small.height()), (15, 7));
        assert!(small.pixels().chunks(4).all(|p| p == [200, 100, 50, 255]));
        // A one-pixel checkerboard at a non-integer ratio comes out grey,
        // not the stripes a sampling filter leaves.
        let checks = picture(30, 30, |x, y| {
            if (x + y) % 2 == 0 { [255, 255, 255, 255] } else { [0, 0, 0, 255] }
        });
        let grey = area_resize(&checks, 11, 11);
        assert!(grey.pixels().chunks(4).all(|p| (100..=156).contains(&p[0])), "{:?}", &grey.pixels()[..16]);
    }

    #[test]
    fn a_preview_region_sits_at_the_centre_and_comes_out_no_larger_than_asked() {
        let region = centre_region(1920, 1080, 2.0, 640, 360);
        assert_eq!(
            region,
            Tile {
                x: 800,
                y: 450,
                width: 320,
                height: 180
            }
        );
        // At once, more of the picture fits the same output.
        assert_eq!(centre_region(1920, 1080, 1.0, 640, 360).width, 640);
        // A picture smaller than the region is taken whole.
        assert_eq!(
            centre_region(200, 100, 2.0, 640, 360),
            Tile {
                x: 0,
                y: 0,
                width: 200,
                height: 100
            }
        );
        let frame = picture(8, 6, |x, y| [x as u8, y as u8, 0, 255]);
        let region = centre_region(8, 6, 1.0, 4, 2);
        let piece = cut(&frame, region);
        assert_eq!((piece.width(), piece.height()), (4, 2));
        assert_eq!(piece.pixel(0, 0), Some([2, 2, 0, 255]));
        assert_eq!(piece.pixel(3, 1), Some([5, 3, 0, 255]));
    }

    #[test]
    fn a_bicubic_enlargement_keeps_a_flat_picture_flat_and_lines_up_its_pixels() {
        let flat = picture(5, 3, |_, _| [40, 120, 200, 255]);
        let big = resize(&flat, 20, 12);
        assert_eq!((big.width(), big.height()), (20, 12));
        assert!(
            big.pixels()
                .chunks_exact(4)
                .all(|p| p == [40, 120, 200, 255]),
            "weights that sum to one leave a flat field alone"
        );
        // The same size is a copy.
        let ramp = picture(6, 4, |x, y| [(x * 40) as u8, (y * 60) as u8, 9, 255]);
        assert_eq!(resize(&ramp, 6, 4).pixels(), ramp.pixels());
        // Doubled, a ramp stays a ramp: output pixel 5 sits at source 2.25,
        // which on a line of 40 a pixel is 90, and the cubic reproduces a
        // line exactly away from the ends. At the ends, where the edge
        // repeats and the line turns flat, it overshoots by a few levels
        // and no more.
        let big = resize(&ramp, 12, 8);
        let reds: Vec<u8> = (0..12).map(|x| big.pixel(x, 3).unwrap()[0]).collect();
        assert!(reds.windows(2).all(|pair| pair[0] <= pair[1]), "{reds:?}");
        assert_eq!(reds[5], 90, "{reds:?}");
        assert_eq!(reds[0], 0);
        assert!((200..=206).contains(&reds[11]), "{reds:?}");
    }

    #[test]
    fn strength_blends_from_the_plain_enlargement_to_the_models_answer() {
        let enhanced = picture(2, 1, |_, _| [200, 100, 0, 255]);
        let plain = picture(2, 1, |_, _| [100, 100, 255, 255]);
        assert_eq!(blend(&enhanced, &plain, 1.0).pixels(), enhanced.pixels());
        assert_eq!(blend(&enhanced, &plain, 0.0).pixels(), plain.pixels());
        assert_eq!(
            blend(&enhanced, &plain, 0.75).pixel(1, 0),
            Some([175, 100, 64, 255])
        );
        // Out of range is held to it.
        assert_eq!(blend(&enhanced, &plain, 3.0).pixels(), enhanced.pixels());
        // Mixing at full strength is the answer untouched; below it, the
        // original is enlarged to the answer's size first.
        let original = picture(1, 1, |_, _| [100, 100, 255, 255]);
        let full = mix(enhanced.clone(), &original, 1.0);
        assert_eq!(full.pixels(), enhanced.pixels());
        let half = mix(enhanced.clone(), &original, 0.5);
        assert_eq!((half.width(), half.height()), (2, 1));
        assert_eq!(half.pixel(0, 0), Some([150, 100, 128, 255]));
    }
}

/// A probe, run by hand: the model through CoreML as Apple's newer
/// MLProgram, its shape fixed so the Neural Engine can take it, timed a
/// window at a time against what the GPU upscaler does.
#[cfg(all(test, feature = "infer", target_vendor = "apple"))]
mod coreml_probe {
    use ort::ep::coreml::{ComputeUnits, ModelFormat, SpecializationStrategy};

    #[test]
    #[ignore = "a probe; needs the fetched model"]
    fn coreml_on_the_neural_engine() {
        let Some(file) = crate::compact::tests_support::fetched_model() else {
            eprintln!("no model here");
            return;
        };
        for (units, side) in [
            (ComputeUnits::CPUAndGPU, 288usize),
            (ComputeUnits::CPUAndGPU, 544),
            (ComputeUnits::CPUAndGPU, 1056),
            (ComputeUnits::All, 544),
        ] {
            let ep = ort::ep::CoreML::default()
                .with_model_format(ModelFormat::MLProgram)
                .with_static_input_shapes(true)
                .with_specialization_strategy(SpecializationStrategy::FastPrediction)
                .with_compute_units(units)
                .build();
            let mut session = ort::session::Session::builder()
                .expect("a builder")
                .with_dimension_override("height", side as i64)
                .expect("height")
                .with_dimension_override("width", side as i64)
                .expect("width")
                .with_execution_providers([ep])
                .expect("coreml")
                .commit_from_file(&file)
                .expect("loads");
            let planes = vec![0.5f32; 3 * side * side];
            let input = || {
                ort::value::Tensor::from_array(([1usize, 3, side, side], planes.clone()))
                    .expect("a tensor")
            };
            let started = std::time::Instant::now();
            session.run(ort::inputs!["input" => input()]).expect("runs");
            let first = started.elapsed();
            let runs = 10;
            let started = std::time::Instant::now();
            for _ in 0..runs {
                session.run(ort::inputs!["input" => input()]).expect("runs");
            }
            let each = started.elapsed().as_secs_f64() / f64::from(runs);
            let core = (side - 32) as f64;
            let windows = (1920.0 / core).ceil() * (1080.0 / core).ceil();
            eprintln!(
                "{units:?} {side}: first {first:?}, then {:.1} ms a window = {:.2} s a 1080p frame",
                each * 1000.0,
                each * windows
            );
        }
    }
}
