// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Decoded image buffers.
//!
//! One pixel format, everywhere: 8-bit RGBA, straight (non-premultiplied)
//! alpha, sRGB, rows packed tightly with no padding. Every decoder converts to
//! it and every encoder converts from it.
//!
//! That is a deliberate simplification, not an oversight. A real grade needs
//! higher precision and a linear working space, and when that day comes it
//! arrives as a second frame type next to this one - not as a `format` field
//! that every call site has to branch on.

use std::fmt;

/// Bytes per pixel in the one format WolfCut uses.
pub const BYTES_PER_PIXEL: usize = 4;

/// An RGBA8 image.
///
/// Width and height are fixed at construction, and the buffer is always
/// exactly `width * height * 4` bytes, so indexing arithmetic cannot go wrong
/// after the fact.
#[derive(Clone, PartialEq, Eq)]
pub struct Frame {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Frame {
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
        Self { width, height, pixels }
    }

    /// A fully transparent frame.
    pub fn transparent(width: u32, height: u32) -> Self {
        Self { width, height, pixels: vec![0u8; Self::byte_len(width, height)] }
    }

    /// Wraps an existing buffer.
    ///
    /// Returns `None` if the buffer is not exactly `width * height * 4` bytes.
    /// Decoders should use this rather than trusting their own arithmetic.
    pub fn from_rgba(width: u32, height: u32, pixels: Vec<u8>) -> Option<Self> {
        (pixels.len() == Self::byte_len(width, height)).then_some(Self { width, height, pixels })
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

    /// The raw RGBA bytes, mutably.
    pub fn pixels_mut(&mut self) -> &mut [u8] {
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

    /// Reads one pixel as `[r, g, b, a]`.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        let offset = self.offset_of(x, y)?;
        Some([
            self.pixels[offset],
            self.pixels[offset + 1],
            self.pixels[offset + 2],
            self.pixels[offset + 3],
        ])
    }

    /// Writes one pixel. Out-of-bounds writes are ignored.
    pub fn set_pixel(&mut self, x: u32, y: u32, rgba: [u8; 4]) {
        if let Some(offset) = self.offset_of(x, y) {
            self.pixels[offset..offset + BYTES_PER_PIXEL].copy_from_slice(&rgba);
        }
    }

    /// Fills the whole frame with one colour.
    pub fn fill(&mut self, rgba: [u8; 4]) {
        for pixel in self.pixels.chunks_exact_mut(BYTES_PER_PIXEL) {
            pixel.copy_from_slice(&rgba);
        }
    }

    /// Iterates over rows, top to bottom.
    pub fn rows(&self) -> impl Iterator<Item = &[u8]> {
        self.pixels.chunks_exact(self.width as usize * BYTES_PER_PIXEL)
    }
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never dump several megabytes of pixels into a log line.
        f.debug_struct("Frame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.pixels.len())
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
}
