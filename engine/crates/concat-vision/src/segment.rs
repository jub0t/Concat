// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Finding the person in a picture.
//!
//! The model is MediaPipe's selfie segmentation, compiled into the binary
//! (see `models/NOTICE.md`) and run by tract. It takes a 256 × 256 RGB
//! picture and answers with a probability per pixel; a source of any shape
//! is squashed to the square on the way in and the mask is read as covering
//! the whole source on the way out, which is the convention every other
//! part of this crate keeps.
//!
//! One inference is about twenty-five milliseconds on a laptop core, and a
//! loaded [`Segmenter`] is shared between threads, so a host that has
//! several cores analyses several frames at once against one model.

use concat_core::frame::{BYTES_PER_PIXEL, Frame};
use tract_onnx::prelude::*;

use crate::{MODEL_SIZE, Mask};

/// The model as the crate ships it.
const MODEL: &[u8] = include_bytes!("../models/selfie-segmentation.onnx");

type Plan = std::sync::Arc<TypedRunnableModel>;

/// A loaded model, ready to run. `Send + Sync`: one per process is enough.
pub struct Segmenter {
    plan: Plan,
}

impl Segmenter {
    /// Loads and optimises the compiled-in model. A few tens of
    /// milliseconds, once.
    pub fn load() -> Result<Segmenter, String> {
        let onnx = tract_onnx::onnx();
        let mut proto = onnx
            .proto_model_for_read(&mut std::io::Cursor::new(MODEL))
            .map_err(|error| format!("cutout model: {error}"))?;
        // The conversion annotates every tensor with a symbolic batch, and
        // the batch here is always one; the annotations would only argue.
        if let Some(graph) = proto.graph.as_mut() {
            graph.value_info.clear();
        }
        let size = MODEL_SIZE as usize;
        let plan = onnx
            .model_for_proto_model(&proto)
            .and_then(|model| model.with_input_fact(0, f32::fact([1, 3, size, size]).into()))
            .and_then(|model| model.into_optimized())
            .and_then(|model| model.into_runnable())
            .map_err(|error| format!("cutout model: {error}"))?;
        Ok(Segmenter { plan })
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
        let input = tract_ndarray::Array4::from_shape_fn((1, 3, size, size), |(_, c, y, x)| {
            // Nearest sample: the decoder already scaled to the square in
            // the analysis path, and a still is fine at this resolution.
            let sx = (x * width / size).min(width - 1);
            let sy = (y * height / size).min(height - 1);
            f32::from(pixels[(sy * width + sx) * BYTES_PER_PIXEL + c]) / 255.0
        });
        let outputs = self
            .plan
            .run(tvec!(input.into_tensor().into()))
            .map_err(|error| format!("cutout: {error}"))?;
        let view = outputs
            .first()
            .ok_or_else(|| "cutout: the model answered nothing".to_owned())?
            .to_plain_array_view::<f32>()
            .map_err(|error| format!("cutout: {error}"))?;
        let data: Vec<u8> = view
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
