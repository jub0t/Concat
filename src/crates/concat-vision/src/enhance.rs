// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

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
//! Four times is more than a clip usually wants - a phone's 1080p is
//! plenty at 4K, and 8K is a file nobody can play - so [`factor_for`]
//! picks two, or one when the picture is already large, and the four-times
//! answer is averaged down to it. Averaging down from the restored picture
//! keeps the restoration and drops only the size.

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
/// The longest side Enhance will produce: past this the picture is
/// restored at its own size rather than enlarged.
pub const LONGEST_SIDE: u32 = 4096;

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

/// How many times larger a `width` by `height` picture comes out: twice,
/// unless that would pass [`LONGEST_SIDE`], and then once, restored at
/// its own size.
pub fn factor_for(width: u32, height: u32) -> u32 {
    if width.max(height) * 2 > LONGEST_SIDE {
        1
    } else {
        2
    }
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

/// The model, loaded once and run tile by tile.
#[cfg(feature = "infer")]
pub struct Enhancer {
    model: std::sync::Mutex<crate::runtime::Model>,
}

#[cfg(feature = "infer")]
impl Enhancer {
    /// Loads the model from its file; see [`crate::models::ModelId::Enhance`].
    pub fn load(file: &std::path::Path) -> Result<Enhancer, String> {
        Ok(Enhancer {
            model: std::sync::Mutex::new(crate::runtime::Model::from_file(file)?),
        })
    }

    /// `frame` restored at [`SCALE`] times its size.
    pub fn upscale(&self, frame: &Frame) -> Result<Frame, String> {
        let (width, height) = (frame.width(), frame.height());
        if width == 0 || height == 0 {
            return Err("enhance: an empty picture".to_owned());
        }
        let mut out = Frame::black(width * SCALE, height * SCALE);
        let side = WINDOW as usize;
        let mut model = self.model.lock().map_err(|_| "enhance: model poisoned")?;
        for tile in tiles(width, height) {
            let planes = window_planes(frame, tile);
            let answer = model.run(
                vec![crate::runtime::Input {
                    name: "input",
                    dims: vec![1, 3, side, side],
                    data: planes.into(),
                }],
                &["output"],
            )?;
            paste(&mut out, tile, &answer[0].data)?;
        }
        Ok(out)
    }

    /// `frame` restored and enlarged `factor` times, `factor` being 1, 2
    /// or 4: the model's answer, averaged down to size.
    pub fn enhance(&self, frame: &Frame, factor: u32) -> Result<Frame, String> {
        let factor = factor.clamp(1, SCALE);
        let large = self.upscale(frame)?;
        Ok(downscale(&large, SCALE / factor))
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
    fn twice_unless_that_would_be_more_than_anyone_can_play() {
        assert_eq!(factor_for(1920, 1080), 2);
        assert_eq!(factor_for(1080, 1920), 2);
        assert_eq!(factor_for(2048, 1152), 2);
        assert_eq!(factor_for(2049, 1152), 1);
        assert_eq!(factor_for(3840, 2160), 1);
        assert_eq!(factor_for(0, 0), 2);
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
}
