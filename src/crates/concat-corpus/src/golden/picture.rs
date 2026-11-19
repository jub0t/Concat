// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The picture a golden is: eight-bit RGBA, straight alpha, as the
//! compositor hands a frame back before any encoder has had it; and the
//! PNG it is kept as.
//!
//! A reference lives in git, without LFS, for as long as its case does, so
//! its file is written to be small and to be the same bytes every time the
//! same pixels are written: an opaque picture drops its alpha channel, and
//! each of PNG's row filters is tried, alone and chosen row by row, with
//! the smallest result kept. A reference blessed twice from the same
//! pixels is then no change at all to the repository.

use std::path::Path;

use concat_core::frame::Frame;

/// An eight-bit RGBA picture, straight alpha, rows from the top.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgba8Image {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Rgba8Image {
    /// Wraps `pixels`, which must be exactly `width * height * 4` bytes of
    /// a picture at least one pixel each way.
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Option<Self> {
        let bytes = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;
        (bytes > 0 && pixels.len() == bytes).then_some(Self {
            width,
            height,
            pixels,
        })
    }

    /// A frame's picture: a deep frame's at eight bits, as
    /// [`Frame::pixel`] reads it.
    pub fn from_frame(frame: &Frame) -> Option<Self> {
        let mut pixels = Vec::with_capacity(frame.width() as usize * frame.height() as usize * 4);
        for y in 0..frame.height() {
            for x in 0..frame.width() {
                pixels.extend_from_slice(&frame.pixel(x, y)?);
            }
        }
        Self::new(frame.width(), frame.height(), pixels)
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The RGBA bytes, row by row from the top left.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// No pixel lets anything through.
    pub fn opaque(&self) -> bool {
        self.pixels.chunks_exact(4).all(|pixel| pixel[3] == 255)
    }

    /// The picture as a person sees it, three bytes a pixel: over a mid
    /// grey where it is not opaque, so that what a clear pixel's colour
    /// channels happen to hold, which nobody sees, is not compared.
    pub(crate) fn seen(&self) -> Vec<u8> {
        const GREY: u32 = 128;
        let mut rgb = Vec::with_capacity(self.pixels.len() / 4 * 3);
        for pixel in self.pixels.chunks_exact(4) {
            let alpha = u32::from(pixel[3]);
            for &level in &pixel[..3] {
                let over = (u32::from(level) * alpha + GREY * (255 - alpha) + 127) / 255;
                rgb.push(over as u8);
            }
        }
        rgb
    }

    /// An opaque picture from three bytes a pixel.
    pub(crate) fn from_rgb(width: u32, height: u32, rgb: &[u8]) -> Option<Self> {
        let pixels = rgb
            .chunks_exact(3)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect();
        Self::new(width, height, pixels)
    }

    /// Reads an eight-bit RGB or RGBA PNG.
    pub fn read_png(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        Self::decode_png(&bytes).map_err(|error| format!("{}: {error}", path.display()))
    }

    /// Decodes an eight-bit RGB or RGBA PNG.
    pub fn decode_png(bytes: &[u8]) -> Result<Self, String> {
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder
            .read_info()
            .map_err(|error| format!("not a PNG: {error}"))?;
        let mut data = vec![0; reader.output_buffer_size()];
        let info = reader
            .next_frame(&mut data)
            .map_err(|error| format!("the PNG does not decode: {error}"))?;
        data.truncate(info.buffer_size());
        if info.bit_depth != png::BitDepth::Eight {
            return Err(format!("the PNG is {:?}, not eight bits", info.bit_depth));
        }
        let picture = match info.color_type {
            png::ColorType::Rgba => Self::new(info.width, info.height, data),
            png::ColorType::Rgb => Self::from_rgb(info.width, info.height, &data),
            other => return Err(format!("the PNG is {other:?}, not RGB or RGBA")),
        };
        picture.ok_or_else(|| "the PNG's size does not match its pixels".to_owned())
    }

    /// The picture as a PNG: the smallest of every row filter's, the same
    /// bytes for the same pixels every time, RGB when the picture is
    /// opaque.
    pub fn encode_png(&self) -> Result<Vec<u8>, String> {
        use png::{AdaptiveFilterType, FilterType};
        let (colour, data) = if self.opaque() {
            let rgb: Vec<u8> = self
                .pixels
                .chunks_exact(4)
                .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
                .collect();
            (png::ColorType::Rgb, rgb)
        } else {
            (png::ColorType::Rgba, self.pixels.clone())
        };
        let tries = [
            (FilterType::Paeth, AdaptiveFilterType::Adaptive),
            (FilterType::NoFilter, AdaptiveFilterType::NonAdaptive),
            (FilterType::Sub, AdaptiveFilterType::NonAdaptive),
            (FilterType::Up, AdaptiveFilterType::NonAdaptive),
            (FilterType::Avg, AdaptiveFilterType::NonAdaptive),
            (FilterType::Paeth, AdaptiveFilterType::NonAdaptive),
        ];
        let mut smallest: Option<Vec<u8>> = None;
        for (filter, adaptive) in tries {
            let mut bytes = Vec::new();
            let mut encoder = png::Encoder::new(&mut bytes, self.width, self.height);
            encoder.set_color(colour);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_compression(png::Compression::Best);
            encoder.set_filter(filter);
            encoder.set_adaptive_filter(adaptive);
            let fail = |error: png::EncodingError| format!("could not encode the PNG: {error}");
            let mut writer = encoder.write_header().map_err(fail)?;
            writer.write_image_data(&data).map_err(fail)?;
            writer.finish().map_err(fail)?;
            // Strictly smaller, so a tie goes to the earlier try and the
            // choice does not depend on anything but the pixels.
            if smallest
                .as_ref()
                .is_none_or(|best| bytes.len() < best.len())
            {
                smallest = Some(bytes);
            }
        }
        smallest.ok_or_else(|| "could not encode the PNG".to_owned())
    }

    /// Writes the picture as a PNG at `path`, making the folder it goes in.
    /// How many bytes were written.
    pub fn write_png(&self, path: &Path) -> Result<u64, String> {
        let bytes = self.encode_png()?;
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)
                .map_err(|error| format!("could not make {}: {error}", folder.display()))?;
        }
        std::fs::write(path, &bytes)
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;
        Ok(bytes.len() as u64)
    }
}

/// The difference between two pictures of one size, made visible: each
/// channel's absolute difference times `gain`, opaque. The gain is the
/// one that takes the largest difference there is to full scale, so a
/// difference of one level in one pixel shows as plainly as a wrong
/// picture does (and one where there is no difference to show); it is
/// returned, for the caption.
pub(crate) fn difference(
    reference: &[u8],
    got: &[u8],
    width: u32,
    height: u32,
) -> Option<(Rgba8Image, u32)> {
    let largest = reference
        .iter()
        .zip(got)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    let gain = match largest {
        0 => 1,
        largest => 255 / u32::from(largest),
    };
    let rgb: Vec<u8> = reference
        .iter()
        .zip(got)
        .map(|(a, b)| (u32::from(a.abs_diff(*b)) * gain).min(255) as u8)
        .collect();
    Some((Rgba8Image::from_rgb(width, height, &rgb)?, gain))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(width: u32, height: u32, alpha: u8) -> Rgba8Image {
        let mut pixels = Vec::new();
        for y in 0..height {
            for x in 0..width {
                pixels.extend_from_slice(&[
                    (x * 255 / width) as u8,
                    (y * 255 / height) as u8,
                    90,
                    alpha,
                ]);
            }
        }
        Rgba8Image::new(width, height, pixels).unwrap()
    }

    #[test]
    fn a_buffer_of_the_wrong_length_or_no_size_is_not_a_picture() {
        assert!(Rgba8Image::new(2, 2, vec![0; 16]).is_some());
        assert!(Rgba8Image::new(2, 2, vec![0; 15]).is_none());
        assert!(Rgba8Image::new(0, 2, Vec::new()).is_none());
    }

    #[test]
    fn a_png_comes_back_as_the_pixels_that_went_in_with_or_without_alpha() {
        for alpha in [255, 200] {
            let picture = gradient(40, 24, alpha);
            let bytes = picture.encode_png().unwrap();
            assert_eq!(Rgba8Image::decode_png(&bytes).unwrap(), picture);
        }
    }

    #[test]
    fn the_same_pixels_are_the_same_bytes_and_an_opaque_picture_is_written_as_rgb() {
        let picture = gradient(64, 36, 255);
        let first = picture.encode_png().unwrap();
        assert_eq!(first, picture.clone().encode_png().unwrap());
        // Colour type is the tenth byte of IHDR's data: 2 is RGB, 6 RGBA.
        assert_eq!(first[25], 2);
        assert_eq!(gradient(64, 36, 254).encode_png().unwrap()[25], 6);
    }

    #[test]
    fn what_is_not_a_png_is_an_error_that_says_so() {
        let error = Rgba8Image::decode_png(b"not a picture").unwrap_err();
        assert!(error.contains("not a PNG"), "{error}");
        // A grey PNG is a PNG, but not a reference.
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
        encoder.set_color(png::ColorType::Grayscale);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[0, 1, 2, 3]).unwrap();
        writer.finish().unwrap();
        let error = Rgba8Image::decode_png(&bytes).unwrap_err();
        assert!(error.contains("not RGB or RGBA"), "{error}");
    }

    #[test]
    fn a_clear_pixel_is_seen_as_the_grey_behind_it_whatever_its_colour() {
        let picture = Rgba8Image::new(2, 1, vec![255, 0, 0, 0, 10, 20, 30, 255]).unwrap();
        assert_eq!(picture.seen(), [128, 128, 128, 10, 20, 30]);
    }

    #[test]
    fn the_difference_picture_takes_the_largest_difference_to_full_scale() {
        let reference = [10, 10, 10, 10, 10, 10];
        let got = [10, 11, 10, 10, 10, 13];
        let (picture, gain) = difference(&reference, &got, 2, 1).unwrap();
        assert_eq!(gain, 85);
        assert_eq!(picture.pixels(), [0, 85, 0, 255, 0, 0, 255, 255]);
    }
}
