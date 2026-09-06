// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Finding the person in a picture.
//!
//! The model is MediaPipe's selfie segmentation, compiled into the binary
//! (see `models/NOTICE.md`) and run by ONNX Runtime. It takes a 256 × 256
//! RGB picture and answers with a probability per pixel; a source of any
//! shape is squashed to the square on the way in and the mask is read as
//! covering the whole source on the way out, which is the convention every
//! other part of this crate keeps.
//!
//! One inference is a few milliseconds with the platform's accelerator and
//! about twenty on a laptop's cores without.

use std::sync::Mutex;

use concat_core::frame::{BYTES_PER_PIXEL, Frame};

use crate::runtime::{Input, Model};
use crate::{MODEL_SIZE, Mask};

/// The model as the crate ships it.
const MODEL: &[u8] = include_bytes!("../models/selfie-segmentation.onnx");

/// A loaded model, ready to run. `Send + Sync`: one per process is enough,
/// and the runtime's own threads take a frame across the cores.
pub struct Segmenter {
    model: Mutex<Model>,
}

impl Segmenter {
    /// Loads the compiled-in model. A few tens of milliseconds, once.
    pub fn load() -> Result<Segmenter, String> {
        Ok(Segmenter {
            model: Mutex::new(Model::from_bytes(MODEL)?),
        })
    }

    /// The person mask of `frame`, at the model's resolution. A frame of
    /// any size: it is resampled to the square first.
    pub fn mask(&self, frame: &Frame) -> Result<Mask, String> {
        let size = MODEL_SIZE as usize;
        let width = frame.width() as usize;
        let height = frame.height() as usize;
        if width == 0 || height == 0 {
            return Err("cutout: an empty frame".to_owned());
        }
        let pixels = frame.pixels();
        let mut data = vec![0f32; 3 * size * size];
        for c in 0..3 {
            for y in 0..size {
                // Nearest sample: the decoder already scaled to the square
                // in the analysis path, and a still is fine at this size.
                let sy = (y * height / size).min(height - 1);
                for x in 0..size {
                    let sx = (x * width / size).min(width - 1);
                    data[(c * size + y) * size + x] =
                        f32::from(pixels[(sy * width + sx) * BYTES_PER_PIXEL + c]) / 255.0;
                }
            }
        }
        let input = Input {
            name: "pixel_values",
            dims: vec![1, 3, size, size],
            data,
        };
        let outputs = self
            .model
            .lock()
            .map_err(|_| "cutout model poisoned".to_owned())?
            .run(vec![input], &["alphas"])?;
        let data: Vec<u8> = outputs[0]
            .data
            .iter()
            .take(size * size)
            .map(|&p: &f32| (p.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
            .collect();
        Mask::from_bytes(MODEL_SIZE, MODEL_SIZE, data)
            .ok_or_else(|| "cutout: the model's answer is the wrong size".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_model_loads_and_answers_a_mask_of_its_size() {
        let segmenter = Segmenter::load().expect("loads");
        // A flat grey picture is nobody: the mask should lean background.
        let mut frame = Frame::black(64, 48);
        for pixel in frame.pixels_mut().chunks_exact_mut(4) {
            pixel[..3].copy_from_slice(&[128, 128, 128]);
        }
        let mask = segmenter.mask(&frame).expect("runs");
        assert_eq!((mask.width(), mask.height()), (MODEL_SIZE, MODEL_SIZE));
        let mean = mask.bytes().iter().map(|&v| u32::from(v)).sum::<u32>() / (256 * 256);
        assert!(mean < 128, "a blank picture read as mostly person: {mean}");
    }
}
