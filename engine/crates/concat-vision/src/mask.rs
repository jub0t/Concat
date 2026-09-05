// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The probability picture: how much of each pixel is the subject.

use std::io::Cursor;

/// An eight-bit picture of probabilities, `0` background and `255` subject,
/// covering the whole source picture whatever its shape.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Mask {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl Mask {
    /// A mask of one value throughout.
    pub fn filled(width: u32, height: u32, value: u8) -> Mask {
        Mask {
            width: width.max(1),
            height: height.max(1),
            data: vec![value; width.max(1) as usize * height.max(1) as usize],
        }
    }

    /// Wraps a buffer of exactly `width * height` bytes.
    pub fn from_bytes(width: u32, height: u32, data: Vec<u8>) -> Option<Mask> {
        (width > 0 && height > 0 && data.len() == width as usize * height as usize).then_some(
            Mask {
                width,
                height,
                data,
            },
        )
    }

    /// Pixels across.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Pixels down.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The bytes, row-major from the top-left.
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    /// The bytes, to write into.
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// The value at a pixel, clamped to the edge.
    pub fn at(&self, x: i64, y: i64) -> u8 {
        let x = x.clamp(0, i64::from(self.width) - 1) as usize;
        let y = y.clamp(0, i64::from(self.height) - 1) as usize;
        self.data[y * self.width as usize + x]
    }

    /// The value at a fraction of the picture, bilinear between the four
    /// pixels around it, in `0..=1`. Outside the picture is the edge.
    pub fn sample(&self, u: f32, v: f32) -> f32 {
        let x = u.clamp(0.0, 1.0) * self.width as f32 - 0.5;
        let y = v.clamp(0.0, 1.0) * self.height as f32 - 0.5;
        let x0 = x.floor();
        let y0 = y.floor();
        let fx = x - x0;
        let fy = y - y0;
        let (x0, y0) = (x0 as i64, y0 as i64);
        let top = f32::from(self.at(x0, y0)) * (1.0 - fx) + f32::from(self.at(x0 + 1, y0)) * fx;
        let bottom =
            f32::from(self.at(x0, y0 + 1)) * (1.0 - fx) + f32::from(self.at(x0 + 1, y0 + 1)) * fx;
        (top * (1.0 - fy) + bottom * fy) / 255.0
    }

    /// The mask softened: three box blurs of `radius` pixels, which is a
    /// gaussian to the eye at a fraction of the cost. Zero is the mask
    /// itself.
    pub fn blurred(&self, radius: f32) -> Mask {
        let radius = radius.round().max(0.0) as usize;
        if radius == 0 {
            return self.clone();
        }
        let mut out = self.clone();
        let mut scratch = vec![0u8; self.data.len()];
        for _ in 0..3 {
            blur_rows(&out.data, &mut scratch, self.width as usize, radius);
            blur_columns(
                &scratch,
                &mut out.data,
                self.width as usize,
                self.height as usize,
                radius,
            );
        }
        out
    }

    /// The mask as an eight-bit greyscale PNG.
    pub fn to_png(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, self.width, self.height);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            // A failed encode into a Vec is out of memory, and an empty
            // file is what the reader treats as a miss.
            if let Ok(mut writer) = encoder.write_header() {
                let _ = writer.write_image_data(&self.data);
            }
        }
        bytes
    }

    /// A mask read back from [`Mask::to_png`], or from any PNG whose first
    /// channel is the mask. `None` for anything that is not a PNG.
    pub fn from_png(bytes: &[u8]) -> Option<Mask> {
        let mut decoder = png::Decoder::new(Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = decoder.read_info().ok()?;
        let mut buffer = vec![0u8; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buffer).ok()?;
        let channels = info.color_type.samples();
        let count = info.width as usize * info.height as usize;
        let data: Vec<u8> = buffer
            .chunks_exact(channels)
            .take(count)
            .map(|pixel| pixel[0])
            .collect();
        Mask::from_bytes(info.width, info.height, data)
    }
}

/// One horizontal box blur, every row.
fn blur_rows(source: &[u8], out: &mut [u8], width: usize, radius: usize) {
    let window = (2 * radius + 1) as u32;
    for (row_in, row_out) in source.chunks_exact(width).zip(out.chunks_exact_mut(width)) {
        let mut sum: u32 = 0;
        // The window starts centred on the first pixel, with the pixels
        // before the edge reading as the edge.
        for offset in 0..=radius {
            sum += u32::from(row_in[offset.min(width - 1)]);
        }
        sum += u32::from(row_in[0]) * radius as u32;
        for x in 0..width {
            row_out[x] = ((sum + window / 2) / window) as u8;
            let leaving = if x >= radius {
                row_in[x - radius]
            } else {
                row_in[0]
            };
            let arriving = row_in[(x + radius + 1).min(width - 1)];
            sum = sum + u32::from(arriving) - u32::from(leaving);
        }
    }
}

/// One vertical box blur, every column.
fn blur_columns(source: &[u8], out: &mut [u8], width: usize, height: usize, radius: usize) {
    let window = (2 * radius + 1) as u32;
    for x in 0..width {
        let at = |y: usize| source[y.min(height - 1) * width + x];
        let mut sum: u32 = 0;
        for offset in 0..=radius {
            sum += u32::from(at(offset));
        }
        sum += u32::from(at(0)) * radius as u32;
        for y in 0..height {
            out[y * width + x] = ((sum + window / 2) / window) as u8;
            let leaving = if y >= radius { at(y - radius) } else { at(0) };
            let arriving = at(y + radius + 1);
            sum = sum + u32::from(arriving) - u32::from(leaving);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampling_reads_the_pixel_under_a_fraction() {
        let mut mask = Mask::filled(4, 2, 0);
        mask.bytes_mut()[3] = 255; // top-right
        assert!((mask.sample(0.875, 0.25) - 1.0).abs() < 1e-6);
        assert!(mask.sample(0.125, 0.25).abs() < 1e-6);
        // Halfway between two pixels is halfway between their values.
        assert!((mask.sample(0.75, 0.25) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn a_blur_spreads_an_edge_and_keeps_the_mean() {
        let mut mask = Mask::filled(16, 1, 0);
        for x in 8..16 {
            mask.bytes_mut()[x] = 255;
        }
        let soft = mask.blurred(2.0);
        assert!(soft.at(7, 0) > 0 && soft.at(8, 0) < 255);
        assert!(soft.at(0, 0) == 0 && soft.at(15, 0) == 255);
        let sum: u32 = soft.bytes().iter().map(|&v| u32::from(v)).sum();
        assert!((sum as i64 - 8 * 255).abs() < 16 * 3);
    }

    #[test]
    fn a_mask_survives_the_png_round_trip() {
        let mut mask = Mask::filled(5, 3, 7);
        mask.bytes_mut()[6] = 200;
        let back = Mask::from_png(&mask.to_png()).expect("decodes");
        assert_eq!(back, mask);
        assert!(Mask::from_png(b"not a png").is_none());
    }
}
