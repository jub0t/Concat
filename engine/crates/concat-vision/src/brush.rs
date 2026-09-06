// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The thing under a brush stroke.
//!
//! The smart brush and smart eraser do not paint a disc: they name a
//! thing, and the whole of it is kept or dropped. The model that reads
//! the thing is SlimSAM, a small Segment Anything, in two halves. The
//! encoder turns a frame into embeddings once, a few hundred milliseconds
//! on a laptop's cores and a few tens with an accelerator; the decoder
//! turns the stroke's points into the mask of what they lie on, in a few
//! milliseconds, so a second stroke on the same frame costs almost
//! nothing. A frame's embedding is therefore worth keeping between
//! strokes, and [`Embedding`] is what a host caches.
//!
//! The model works on a 1024 square: the frame is scaled to fit by its
//! long edge and the rest is padding. The mask it answers with covers
//! that square; [`Brush::region`] reads it back as a mask over the source
//! picture alone, in the fractions every other mask here uses.

use std::path::Path;
use std::sync::Mutex;

use concat_core::frame::{BYTES_PER_PIXEL, Frame};

use crate::Mask;
use crate::runtime::{Input, Model, Output};

/// The model's square.
const SAM_SIZE: usize = 1024;
/// The decoder's answer is a quarter the size of the square.
const SAM_MASK: usize = 256;
/// The region is stored at this size over the source picture.
const REGION_SIZE: u32 = 256;
/// ImageNet's mean and deviation per channel, the model's normalisation.
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

/// A frame as the encoder saw it: its embeddings, and how much of the
/// square the picture filled.
pub struct Embedding {
    image: Output,
    positional: Output,
    /// The picture's width and height inside the square, in pixels.
    scaled: (f32, f32),
}

/// The two halves of the brush model, loaded.
pub struct Brush {
    encoder: Mutex<Model>,
    decoder: Mutex<Model>,
}

impl Brush {
    /// Loads the encoder and decoder from their downloaded files.
    pub fn load(encoder: &Path, decoder: &Path) -> Result<Brush, String> {
        Ok(Brush {
            encoder: Mutex::new(Model::from_file(encoder)?),
            decoder: Mutex::new(Model::from_file(decoder)?),
        })
    }

    /// The size to decode a source of `width` × `height` to for the
    /// encoder: fitted inside the square by its long edge.
    pub fn input_size(width: u32, height: u32) -> (u32, u32) {
        let (w, h) = (width.max(1) as f64, height.max(1) as f64);
        let scale = SAM_SIZE as f64 / w.max(h);
        (
            ((w * scale).round() as u32).clamp(1, SAM_SIZE as u32),
            ((h * scale).round() as u32).clamp(1, SAM_SIZE as u32),
        )
    }

    /// Reads a frame. The expensive half.
    pub fn embed(&self, frame: &Frame) -> Result<Embedding, String> {
        if frame.width() == 0 || frame.height() == 0 {
            return Err("brush: an empty frame".to_owned());
        }
        let (width, height) = Brush::input_size(frame.width(), frame.height());
        let (width, height) = (width as usize, height as usize);
        let (fw, fh) = (frame.width() as usize, frame.height() as usize);
        let pixels = frame.pixels();
        // Normalised picture in the top-left of the square, zero beyond.
        let mut data = vec![0f32; 3 * SAM_SIZE * SAM_SIZE];
        for c in 0..3 {
            for y in 0..height {
                let sy = (y * fh / height).min(fh - 1);
                for x in 0..width {
                    let sx = (x * fw / width).min(fw - 1);
                    let value = f32::from(pixels[(sy * fw + sx) * BYTES_PER_PIXEL + c]) / 255.0;
                    data[(c * SAM_SIZE + y) * SAM_SIZE + x] = (value - MEAN[c]) / STD[c];
                }
            }
        }
        let input = Input {
            name: "pixel_values",
            dims: vec![1, 3, SAM_SIZE, SAM_SIZE],
            data: data.into(),
        };
        let mut outputs = self
            .encoder
            .lock()
            .map_err(|_| "brush encoder poisoned".to_owned())?
            .run(vec![input], &["image_embeddings", "image_positional_embeddings"])?;
        let positional = outputs.pop().ok_or("brush: no positional embeddings")?;
        let image = outputs.pop().ok_or("brush: no image embeddings")?;
        Ok(Embedding {
            image,
            positional,
            scaled: (width as f32, height as f32),
        })
    }

    /// The thing under `points`, which are source fractions along the
    /// stroke, as a mask over the source picture. The cheap half.
    pub fn region(&self, embedding: &Embedding, points: &[[f64; 2]]) -> Result<Mask, String> {
        // A stroke has as many points as the pointer reported; the model
        // wants a handful. Sixteen spread along the path say where the
        // stroke went without drowning the prompt.
        const PROMPTS: usize = 16;
        let picked: Vec<[f64; 2]> = if points.len() <= PROMPTS {
            points.to_vec()
        } else {
            (0..PROMPTS)
                .map(|i| points[i * (points.len() - 1) / (PROMPTS - 1)])
                .collect()
        };
        if picked.is_empty() {
            return Err("brush: a stroke with no points".to_owned());
        }
        let (sw, sh) = embedding.scaled;
        let coords: Vec<f32> = picked
            .iter()
            .flat_map(|[x, y]| {
                [
                    (*x as f32).clamp(0.0, 1.0) * sw,
                    (*y as f32).clamp(0.0, 1.0) * sh,
                ]
            })
            .collect();
        let labels = vec![1i64; picked.len()];
        let inputs = vec![
            Input {
                name: "input_points",
                dims: vec![1, 1, picked.len(), 2],
                data: coords.into(),
            },
            Input {
                name: "input_labels",
                dims: vec![1, 1, picked.len()],
                data: labels.into(),
            },
            Input {
                name: "image_embeddings",
                dims: embedding.image.dims.clone(),
                data: embedding.image.data.clone().into(),
            },
            Input {
                name: "image_positional_embeddings",
                dims: embedding.positional.dims.clone(),
                data: embedding.positional.data.clone().into(),
            },
        ];
        let outputs = self
            .decoder
            .lock()
            .map_err(|_| "brush decoder poisoned".to_owned())?
            .run(inputs, &["iou_scores", "pred_masks"])?;
        // Three candidate masks, scored; the model's own best guess wins.
        let best = outputs[0]
            .data
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(index, _)| index)
            .unwrap_or(0);
        let masks = &outputs[1].data;
        let plane = SAM_MASK * SAM_MASK;
        if masks.len() < (best + 1) * plane {
            return Err("brush: the model's answer is the wrong size".to_owned());
        }
        let logits = &masks[best * plane..(best + 1) * plane];
        // The answer covers the square; the picture is its top-left
        // corner. Read the region over the picture alone.
        let size = REGION_SIZE as usize;
        let mut bytes = Vec::with_capacity(size * size);
        for y in 0..size {
            let py = ((y as f32 + 0.5) / size as f32) * sh / SAM_SIZE as f32 * SAM_MASK as f32;
            for x in 0..size {
                let px = ((x as f32 + 0.5) / size as f32) * sw / SAM_SIZE as f32 * SAM_MASK as f32;
                let logit = bilinear(logits, SAM_MASK, px - 0.5, py - 0.5);
                let probability = 1.0 / (1.0 + (-logit).exp());
                bytes.push((probability.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
            }
        }
        Mask::from_bytes(REGION_SIZE, REGION_SIZE, bytes)
            .ok_or_else(|| "brush: the region is the wrong size".to_owned())
    }
}

/// A value from a square plane, bilinear between the four around it,
/// clamped to the edge.
fn bilinear(plane: &[f32], edge: usize, x: f32, y: f32) -> f32 {
    let at = |x: i64, y: i64| {
        let x = x.clamp(0, edge as i64 - 1) as usize;
        let y = y.clamp(0, edge as i64 - 1) as usize;
        plane[y * edge + x]
    };
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (x0, y0) = (x0 as i64, y0 as i64);
    let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1, y0) * fx;
    let bottom = at(x0, y0 + 1) * (1.0 - fx) + at(x0 + 1, y0 + 1) * fx;
    top * (1.0 - fy) + bottom * fy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_fits_the_square_by_its_long_edge() {
        assert_eq!(Brush::input_size(1920, 1080), (1024, 576));
        assert_eq!(Brush::input_size(1080, 1920), (576, 1024));
        assert_eq!(Brush::input_size(500, 500), (1024, 1024));
    }

    #[test]
    fn bilinear_reads_between_pixels_and_clamps_at_the_edge() {
        let plane = [0.0, 1.0, 2.0, 3.0];
        assert_eq!(bilinear(&plane, 2, 0.0, 0.0), 0.0);
        assert_eq!(bilinear(&plane, 2, 0.5, 0.0), 0.5);
        assert_eq!(bilinear(&plane, 2, 0.5, 0.5), 1.5);
        assert_eq!(bilinear(&plane, 2, 5.0, 5.0), 3.0);
    }
}
