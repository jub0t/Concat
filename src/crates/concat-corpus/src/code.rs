// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The code every generated picture carries: which media it is and which
//! of that media's frames, written in the picture itself, so a frame read
//! back from an export names its own source and the test needs no stored
//! reference to say whether it is the right one.
//!
//! The code is twenty blocks of 16 × 16 pixels in two rows of ten, at the
//! picture's top left corner: 160 × 32 pixels in all. Each block is grey
//! at one of two video levels, 16 for a nought and 235 for a one, so it
//! lives in the luma alone and the halved chroma of a 4:2:0 export cannot
//! touch it. Read row by row, the twenty bits are:
//!
//! | bits   | what |
//! |--------|------|
//! | 0, 1   | start marker: one, nought |
//! | 2-4    | media tag, 1-7 (nought is never written) |
//! | 5-17   | source frame index, 0-8191 |
//! | 18     | parity: the ones in bits 2-18 come to an even count |
//! | 19     | end marker: one |
//!
//! Two rows of ten rather than one row of twenty so that the code fits a
//! 4:3 still placed unscaled on a 16:9 frame (240 pixels wide at 320 × 180)
//! as well as it fits the frame itself, and reads the same way off both.
//!
//! A read is refused rather than guessed. A block is the mean luma of its
//! inner 8 × 8, away from the edges a codec smears, and is read by the
//! midpoint between the two levels; but a mean in the middle third, too
//! far from both levels to be either, makes the whole read [`None`], as
//! do wrong markers, a parity that does not hold, and a tag of nought. A
//! blend of two frames, a shifted or scaled picture, a black frame or a
//! white one is therefore reported as unreadable, never as some other
//! frame.

use concat_core::frame::Frame;

/// A block's side, in pixels.
pub const BLOCK: u32 = 16;
/// Blocks across one row of the code.
pub const COLUMNS: u32 = 10;
/// Rows of blocks.
pub const ROWS: u32 = 2;
/// The code's width, in pixels.
pub const WIDTH: u32 = BLOCK * COLUMNS;
/// The code's height, in pixels.
pub const HEIGHT: u32 = BLOCK * ROWS;

/// The level a nought is written at: video black.
pub const LOW: u8 = 16;
/// The level a one is written at: video white.
pub const HIGH: u8 = 235;
/// How far from its level a block's mean may be and still be read. A
/// third of the gap between the levels: a block further from both than
/// this is in the middle third and reads as neither.
pub const MARGIN: f64 = (HIGH - LOW) as f64 / 3.0;

/// The highest frame index the code carries.
pub const MAX_INDEX: u32 = (1 << INDEX_BITS) - 1;
/// The highest media tag.
pub const MAX_TAG: u8 = (1 << TAG_BITS) - 1;

const TAG_BITS: u32 = 3;
const INDEX_BITS: u32 = 13;
const BITS: usize = (COLUMNS * ROWS) as usize;

/// What a picture says it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize)]
pub struct Code {
    /// Which media, 1 to [`MAX_TAG`].
    pub tag: u8,
    /// Which of its frames, from nought.
    pub index: u32,
}

impl std::fmt::Display for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "media {} frame {}", self.tag, self.index)
    }
}

/// One read of a code: what it says, if anything, and how cleanly the
/// blocks came through.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Reading {
    /// The code, or None when the read was refused.
    pub code: Option<Code>,
    /// The furthest any block's mean was from the level it was read as, in
    /// 8-bit levels: how much of [`MARGIN`] the encoder used up.
    pub worst: f64,
    /// Every block's mean luma, row by row, for a report on a refused read.
    pub means: Vec<f64>,
}

/// The twenty bits of `code`, row by row.
fn bits_of(code: Code) -> [bool; BITS] {
    assert!(
        (1..=MAX_TAG).contains(&code.tag),
        "a media tag is 1 to {MAX_TAG}, not {}",
        code.tag
    );
    assert!(
        code.index <= MAX_INDEX,
        "a frame index is at most {MAX_INDEX}, not {}",
        code.index
    );
    let mut bits = [false; BITS];
    bits[0] = true;
    bits[1] = false;
    for bit in 0..TAG_BITS {
        bits[2 + bit as usize] = code.tag >> (TAG_BITS - 1 - bit) & 1 == 1;
    }
    for bit in 0..INDEX_BITS {
        bits[5 + bit as usize] = code.index >> (INDEX_BITS - 1 - bit) & 1 == 1;
    }
    let ones = bits[2..18].iter().filter(|bit| **bit).count();
    bits[18] = ones % 2 == 1;
    bits[19] = true;
    bits
}

/// The code the twenty bits say, or None where the markers or the parity
/// do not hold or the tag is nought.
fn code_of(bits: &[bool; BITS]) -> Option<Code> {
    if !bits[0] || bits[1] || !bits[19] {
        return None;
    }
    if bits[2..19].iter().filter(|bit| **bit).count() % 2 != 0 {
        return None;
    }
    let tag = bits[2..5]
        .iter()
        .fold(0u8, |value, bit| value << 1 | u8::from(*bit));
    let index = bits[5..18]
        .iter()
        .fold(0u32, |value, bit| value << 1 | u32::from(*bit));
    (tag != 0).then_some(Code { tag, index })
}

/// Paints `code` into `frame` with its top left corner at (`x`, `y`).
///
/// # Panics
/// If the code does not fit inside the frame there, or the tag or index is
/// out of range.
pub fn write_code(frame: &mut Frame, x: u32, y: u32, code: Code) {
    assert!(
        x + WIDTH <= frame.width() && y + HEIGHT <= frame.height(),
        "a code at ({x}, {y}) does not fit a {}×{} frame",
        frame.width(),
        frame.height()
    );
    let stride = frame.width() as usize * 4;
    let bits = bits_of(code);
    let pixels = frame.pixels_mut();
    for (at, bit) in bits.iter().enumerate() {
        let level = if *bit { HIGH } else { LOW };
        let left = x + (at as u32 % COLUMNS) * BLOCK;
        let top = y + (at as u32 / COLUMNS) * BLOCK;
        for row in top..top + BLOCK {
            for column in left..left + BLOCK {
                let offset = row as usize * stride + column as usize * 4;
                pixels[offset..offset + 4].copy_from_slice(&[level, level, level, 255]);
            }
        }
    }
}

/// Reads the code at the top left corner of `frame`.
pub fn read_code(frame: &Frame) -> Option<Code> {
    read_at(frame, 0, 0).code
}

/// Reads the code whose top left corner is at (`x`, `y`) in `frame`,
/// with how cleanly it came through. A code that would not fit there is a
/// refused read.
pub fn read_at(frame: &Frame, x: u32, y: u32) -> Reading {
    if x + WIDTH > frame.width() || y + HEIGHT > frame.height() {
        return Reading {
            code: None,
            worst: f64::INFINITY,
            means: Vec::new(),
        };
    }
    let midpoint = (f64::from(LOW) + f64::from(HIGH)) / 2.0;
    let mut bits = [false; BITS];
    let mut means = Vec::with_capacity(BITS);
    let mut worst: f64 = 0.0;
    let mut clear = true;
    for (at, bit) in bits.iter_mut().enumerate() {
        // The inner 8 × 8: a quarter of the side in from every edge.
        let left = x + (at as u32 % COLUMNS) * BLOCK + BLOCK / 4;
        let top = y + (at as u32 / COLUMNS) * BLOCK + BLOCK / 4;
        let mut sum = 0.0;
        for row in top..top + BLOCK / 2 {
            for column in left..left + BLOCK / 2 {
                let [r, g, b, _] = frame.pixel(column, row).unwrap_or([0; 4]);
                sum += luma(r, g, b);
            }
        }
        let mean = sum / f64::from(BLOCK / 2 * BLOCK / 2);
        *bit = mean > midpoint;
        let level = if *bit { HIGH } else { LOW };
        let off = (mean - f64::from(level)).abs();
        worst = worst.max(off);
        clear &= off <= MARGIN;
        means.push(mean);
    }
    Reading {
        code: if clear { code_of(&bits) } else { None },
        worst,
        means,
    }
}

/// A pixel's luma, by BT.709's weights: on the grey the code is written in
/// it is the grey itself, and off it the colour a codec bleeds in from a
/// neighbour counts for what it does to brightness, not in full.
fn luma(r: u8, g: u8, b: u8) -> f64 {
    0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn painted(code: Code) -> Frame {
        let mut frame = Frame::black(320, 180);
        frame.fill([90, 140, 200, 255]);
        write_code(&mut frame, 0, 0, code);
        frame
    }

    #[test]
    fn every_tag_and_the_ends_of_the_index_read_back_as_written() {
        for tag in 1..=MAX_TAG {
            for index in [0, 1, 2, 44, 45, 1000, MAX_INDEX - 1, MAX_INDEX] {
                let code = Code { tag, index };
                let reading = read_at(&painted(code), 0, 0);
                assert_eq!(reading.code, Some(code));
                assert_eq!(reading.worst, 0.0);
            }
        }
    }

    #[test]
    fn a_code_reads_where_it_was_written_and_nowhere_else() {
        let code = Code { tag: 3, index: 0 };
        let mut frame = Frame::black(320, 180);
        write_code(&mut frame, 40, 0, code);
        assert_eq!(read_at(&frame, 40, 0).code, Some(code));
        assert_eq!(read_code(&frame), None);
        assert_eq!(read_at(&frame, 48, 0).code, None);
    }

    #[test]
    fn one_flipped_block_is_refused_never_misread() {
        let code = Code {
            tag: 2,
            index: 1234,
        };
        for at in 0..BITS as u32 {
            let mut frame = painted(code);
            let left = (at % COLUMNS) * BLOCK;
            let top = (at / COLUMNS) * BLOCK;
            let [level, ..] = frame.pixel(left, top).unwrap();
            let flipped = if level == HIGH { LOW } else { HIGH };
            for y in top..top + BLOCK {
                for x in left..left + BLOCK {
                    let offset = (y * 320 + x) as usize * 4;
                    frame.pixels_mut()[offset..offset + 3].fill(flipped);
                }
            }
            assert_eq!(read_code(&frame), None, "block {at} flipped");
        }
    }

    #[test]
    fn a_blend_of_two_frames_is_refused_never_misread() {
        let one = painted(Code { tag: 1, index: 44 });
        let two = painted(Code { tag: 1, index: 45 });
        let pixels = one
            .pixels()
            .iter()
            .zip(two.pixels())
            .map(|(a, b)| ((u16::from(*a) + u16::from(*b)) / 2) as u8)
            .collect();
        let blend = Frame::from_rgba(320, 180, pixels).unwrap();
        assert_eq!(read_code(&blend), None);
    }

    #[test]
    fn black_and_white_frames_have_no_code() {
        let mut frame = Frame::black(320, 180);
        assert_eq!(read_code(&frame), None);
        frame.fill([255, 255, 255, 255]);
        assert_eq!(read_code(&frame), None);
        frame.fill([LOW, LOW, LOW, 255]);
        assert_eq!(read_code(&frame), None);
    }

    #[test]
    fn a_code_too_close_to_the_edge_is_refused() {
        let frame = painted(Code { tag: 1, index: 0 });
        assert_eq!(read_at(&frame, 200, 0).code, None);
        assert_eq!(read_at(&frame, 0, 160).code, None);
    }
}
