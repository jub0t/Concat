// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Decoded image buffers.
//!
//! One pixel format almost everywhere: 8-bit RGBA, straight (non-premultiplied)
//! alpha, gamma-encoded Rec. 709, rows packed tightly with no padding. Every
//! encoder converts from it, and every consumer but the compositor's upload
//! reads it.
//!
//! The exception is a deep frame ([`Depth::Sixteen`]): sixteen bits a
//! channel, still RGBA, in the source's own signal ([`Signal`]) - an HDR clip
//! as it was shot, not tone-mapped to eight bits on the CPU. The decoder
//! makes one only when a caller asks (`DecodeOptions::deep`) and the source
//! is wide, and the only caller that asks is the one whose frames go
//! straight to the GPU, which brings them into its working space as it
//! uploads them. Nothing else ever sees one, so no call site branches on
//! the depth: the note that once said a deeper frame would arrive as a
//! type of its own gave way to that, since a second type would have run
//! through the frame pool, the plan and every cache for one reader.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

/// Bytes per pixel of an eight-bit frame, the format nearly everything uses.
pub const BYTES_PER_PIXEL: usize = 4;

/// The widest or tallest a frame may be, a side: 8K. The downlevel
/// texture limit every GPU meets is this, so a frame past it cannot be
/// drawn, only asked for; a document, a request and a preview are all held
/// to it before a texture is made (audit 2026-10-09, finding 8).
pub const MAX_SIDE: u32 = 8192;

/// How many bits a channel a frame's pixels hold.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Depth {
    /// Eight: `u8` channels, four bytes a pixel.
    #[default]
    Eight,
    /// Sixteen: little-endian `u16` channels, eight bytes a pixel.
    Sixteen,
}

/// What a frame's RGB values mean: the transfer they are encoded with and
/// the primaries they are on. An eight-bit frame is always [`Signal::Sdr`];
/// a deep one carries its source's.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Signal {
    /// Gamma-encoded (BT.1886, 2.4) on Rec. 709 primaries.
    #[default]
    Sdr,
    /// Gamma-encoded (BT.1886, 2.4) on Rec. 2020 primaries: a wide-gamut
    /// SDR source.
    SdrWide,
    /// ARIB STD-B67 hybrid log-gamma on Rec. 2020: an iPhone's HDR.
    Hlg,
    /// SMPTE ST 2084 perceptual quantiser on Rec. 2020: HDR10.
    Pq,
}

/// The next frame identity; see [`Frame::id`].
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn mint() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// An RGBA8 image.
///
/// Width and height are fixed at construction, and the buffer is always
/// exactly `width * height * 4` bytes, so indexing arithmetic cannot go wrong
/// after the fact.
pub struct Frame {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    /// Which picture this is; see [`Frame::id`].
    id: u64,
    /// Eight bits a channel, or sixteen for a deep frame.
    depth: Depth,
    /// What the values mean.
    signal: Signal,
}

impl Clone for Frame {
    /// A copy is a picture of its own: it may be changed next, and a
    /// renderer holding the original must not take it for the copy.
    fn clone(&self) -> Self {
        Self {
            width: self.width,
            height: self.height,
            pixels: self.pixels.clone(),
            id: mint(),
            depth: self.depth,
            signal: self.signal,
        }
    }
}

impl PartialEq for Frame {
    /// The same picture, whatever its identity.
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.depth == other.depth
            && self.signal == other.signal
            && self.pixels == other.pixels
    }
}

impl Eq for Frame {}

impl Frame {
    /// A number no other frame has had, and this one loses the moment its
    /// pixels are touched: a renderer keys what it has uploaded by it, so a
    /// picture it already holds - a still, a title, a paused clip's frame
    /// - is not uploaded again. Never zero.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// How many bytes a frame of this size occupies.
    pub const fn byte_len(width: u32, height: u32) -> usize {
        width as usize * height as usize * BYTES_PER_PIXEL
    }

    /// An opaque black frame.
    pub fn black(width: u32, height: u32) -> Self {
        let mut pixels = vec![0u8; Self::byte_len(width, height)];
        for pixel in pixels.chunks_exact_mut(BYTES_PER_PIXEL) {
            pixel[3] = 255;
        }
        Self {
            width,
            height,
            pixels,
            id: mint(),
            depth: Depth::Eight,
            signal: Signal::Sdr,
        }
    }

    /// A fully transparent frame.
    pub fn transparent(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![0u8; Self::byte_len(width, height)],
            id: mint(),
            depth: Depth::Eight,
            signal: Signal::Sdr,
        }
    }

    /// Wraps an existing buffer.
    ///
    /// Returns `None` if the buffer is not exactly `width * height * 4` bytes.
    /// Decoders should use this rather than trusting their own arithmetic.
    pub fn from_rgba(width: u32, height: u32, pixels: Vec<u8>) -> Option<Self> {
        (pixels.len() == Self::byte_len(width, height)).then_some(Self {
            width,
            height,
            pixels,
            id: mint(),
            depth: Depth::Eight,
            signal: Signal::Sdr,
        })
    }

    /// Wraps a deep buffer: little-endian `u16` RGBA, exactly `width *
    /// height * 8` bytes, in `signal`. `None` if the size is wrong.
    pub fn from_rgba64(width: u32, height: u32, pixels: Vec<u8>, signal: Signal) -> Option<Self> {
        (pixels.len() == Self::byte_len(width, height) * 2).then_some(Self {
            width,
            height,
            pixels,
            id: mint(),
            depth: Depth::Sixteen,
            signal,
        })
    }

    /// Eight bits a channel, or sixteen.
    pub const fn depth(&self) -> Depth {
        self.depth
    }

    /// What the values mean: see [`Signal`].
    pub const fn signal(&self) -> Signal {
        self.signal
    }

    /// Bytes a pixel of this frame takes.
    pub const fn bytes_per_pixel(&self) -> usize {
        match self.depth {
            Depth::Eight => BYTES_PER_PIXEL,
            Depth::Sixteen => BYTES_PER_PIXEL * 2,
        }
    }

    /// Width in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// True if both frames have the same dimensions.
    pub const fn same_size_as(&self, other: &Self) -> bool {
        self.width == other.width && self.height == other.height
    }

    /// The raw RGBA bytes, row-major from the top-left.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// The raw RGBA bytes, mutably. A new identity with them: whatever is
    /// written makes this a different picture from the one uploaded.
    pub fn pixels_mut(&mut self) -> &mut [u8] {
        self.id = mint();
        &mut self.pixels
    }

    /// Consumes the frame and yields its buffer, so it can be handed to an
    /// encoder without a copy.
    pub fn into_pixels(self) -> Vec<u8> {
        self.pixels
    }

    /// Byte offset of the pixel at `(x, y)`, or `None` if out of bounds.
    pub const fn offset_of(&self, x: u32, y: u32) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        Some((y as usize * self.width as usize + x as usize) * BYTES_PER_PIXEL)
    }

    /// Reads one pixel as `[r, g, b, a]`: a deep frame's to eight bits, its
    /// channels' high bytes, for a test or a log to look at.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        let offset = self.offset_of(x, y)?;
        Some(match self.depth {
            Depth::Eight => [
                self.pixels[offset],
                self.pixels[offset + 1],
                self.pixels[offset + 2],
                self.pixels[offset + 3],
            ],
            Depth::Sixteen => {
                let at = offset * 2;
                [
                    self.pixels[at + 1],
                    self.pixels[at + 3],
                    self.pixels[at + 5],
                    self.pixels[at + 7],
                ]
            }
        })
    }

    /// Writes one pixel. Out-of-bounds writes are ignored.
    ///
    /// A deep frame keeps its depth: each channel's byte lands in the high
    /// half of its `u16`, the byte [`Frame::pixel`] reads back, so what was
    /// written is what is read. A four-byte write at the 8-bit offset, as a
    /// blind RGBA8 store would do, lands inside a different pixel's
    /// channels and scrambles two at once.
    pub fn set_pixel(&mut self, x: u32, y: u32, rgba: [u8; 4]) {
        self.id = mint();
        if let Some(offset) = self.offset_of(x, y) {
            match self.depth {
                Depth::Eight => {
                    self.pixels[offset..offset + BYTES_PER_PIXEL].copy_from_slice(&rgba);
                }
                Depth::Sixteen => {
                    let at = offset * 2;
                    for (channel, value) in rgba.into_iter().enumerate() {
                        self.pixels[at + channel * 2 + 1] = value;
                    }
                }
            }
        }
    }

    /// Copies `source` into this frame with its top-left corner at (`x`, `y`),
    /// clipping whatever falls outside. No blending: the source's pixels
    /// replace what was there.
    ///
    /// Both frames must hold the same depth: a blit is a byte copy, and an
    /// eight-bit frame's pixels are half a deep one's, so a copy between
    /// the two would have to invent values - widening, narrowing - that
    /// belong to a conversion pass, not to a blit. A mismatched pair is
    /// ignored rather than copied wrong; no call site mixes the two, since
    /// a deep frame goes straight to the GPU and never meets a compositor.
    pub fn blit(&mut self, source: &Frame, x: u32, y: u32) {
        if self.depth != source.depth {
            return;
        }
        self.id = mint();
        let width = self.width;
        let height = self.height;
        let bytes = self.bytes_per_pixel();
        let columns = source.width.min(width.saturating_sub(x)) as usize;
        let rows = source.height.min(height.saturating_sub(y)) as usize;
        if columns == 0 || rows == 0 {
            return;
        }
        let source_row = source.width as usize * bytes;
        let target_row = width as usize * bytes;
        for row in 0..rows {
            let from = row * source_row;
            let to = (y as usize + row) * target_row + x as usize * bytes;
            self.pixels[to..to + columns * bytes]
                .copy_from_slice(&source.pixels[from..from + columns * bytes]);
        }
    }

    /// Fills the whole frame with one colour, deep frames included: the
    /// colour's channels go where [`Frame::set_pixel`] puts them, so a deep
    /// frame filled reads back the colour from every pixel. Four-byte
    /// chunks alone would cover only half a deep frame, split across every
    /// pixel they touched.
    pub fn fill(&mut self, rgba: [u8; 4]) {
        self.id = mint();
        match self.depth {
            Depth::Eight => {
                for pixel in self.pixels.chunks_exact_mut(BYTES_PER_PIXEL) {
                    pixel.copy_from_slice(&rgba);
                }
            }
            Depth::Sixteen => {
                for pixel in self.pixels.chunks_exact_mut(BYTES_PER_PIXEL * 2) {
                    for (channel, value) in rgba.into_iter().enumerate() {
                        pixel[channel * 2 + 1] = value;
                    }
                }
            }
        }
    }

    /// Iterates over rows, top to bottom: `width * bytes_per_pixel` bytes
    /// each, tightly packed.
    pub fn rows(&self) -> impl Iterator<Item = &[u8]> {
        self.pixels
            .chunks_exact(self.width as usize * self.bytes_per_pixel())
    }
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never dump several megabytes of pixels into a log line.
        f.debug_struct("Frame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.pixels.len())
            .field("depth", &self.depth)
            .field("signal", &self.signal)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_is_opaque() {
        let frame = Frame::black(2, 2);
        assert_eq!(frame.pixel(0, 0), Some([0, 0, 0, 255]));
        assert_eq!(frame.pixels().len(), 16);
    }

    #[test]
    fn transparent_is_clear() {
        assert_eq!(Frame::transparent(1, 1).pixel(0, 0), Some([0, 0, 0, 0]));
    }

    #[test]
    fn rejects_a_buffer_of_the_wrong_size() {
        assert!(Frame::from_rgba(2, 2, vec![0; 15]).is_none());
        assert!(Frame::from_rgba(2, 2, vec![0; 16]).is_some());
    }

    #[test]
    fn out_of_bounds_access_is_none_not_a_panic() {
        let mut frame = Frame::black(2, 2);
        assert_eq!(frame.pixel(2, 0), None);
        assert_eq!(frame.pixel(0, 2), None);
        frame.set_pixel(99, 99, [255, 0, 0, 255]); // ignored
    }

    #[test]
    fn set_and_read_a_pixel() {
        let mut frame = Frame::black(4, 3);
        frame.set_pixel(3, 2, [1, 2, 3, 4]);
        assert_eq!(frame.pixel(3, 2), Some([1, 2, 3, 4]));
        assert_eq!(frame.pixel(2, 3), None);
    }

    #[test]
    fn rows_are_tightly_packed() {
        let frame = Frame::black(5, 4);
        assert_eq!(frame.rows().count(), 4);
        assert!(frame.rows().all(|row| row.len() == 20));
    }

    /// A deep frame's pixels are eight bytes, and the RGBA8 ways in -
    /// `set_pixel`, `fill`, `rows` - honour that instead of writing
    /// four-byte pixels into the middle of it.
    #[test]
    fn a_deep_frame_keeps_its_own_stride() {
        let mut deep = Frame::from_rgba64(2, 1, vec![0u8; 16], Signal::Hlg).expect("deep");
        deep.set_pixel(1, 0, [1, 2, 3, 4]);
        assert_eq!(deep.pixel(1, 0), Some([1, 2, 3, 4]));
        assert_eq!(
            deep.pixel(0, 0),
            Some([0, 0, 0, 0]),
            "its neighbour is untouched"
        );
        // The bytes: each channel in the high half of its u16, the half
        // `pixel` reads.
        assert_eq!(&deep.pixels()[8..16], &[0, 1, 0, 2, 0, 3, 0, 4]);
        deep.fill([9, 8, 7, 6]);
        assert_eq!(deep.pixel(0, 0), Some([9, 8, 7, 6]));
        assert_eq!(deep.pixel(1, 0), Some([9, 8, 7, 6]));
        assert_eq!(deep.rows().count(), 1);
        assert_eq!(deep.rows().next().map(<[u8]>::len), Some(16));
    }

    /// A blit between frames of one depth copies bytes at that depth's
    /// stride, and one between the two depths is ignored rather than
    /// copied wrong.
    #[test]
    fn a_deep_frame_blits_its_own_kind_and_ignores_a_shallow_one() {
        let mut deep = Frame::from_rgba64(2, 1, vec![0u8; 16], Signal::Sdr).expect("deep");
        let source =
            Frame::from_rgba64(1, 1, vec![0, 5, 0, 6, 0, 7, 0, 8], Signal::Sdr).expect("deep");
        deep.blit(&source, 1, 0);
        assert_eq!(deep.pixel(1, 0), Some([5, 6, 7, 8]));
        let before = deep.clone();
        deep.blit(&Frame::black(1, 1), 0, 0);
        assert_eq!(deep.pixels(), before.pixels(), "nothing was copied");
    }
}
