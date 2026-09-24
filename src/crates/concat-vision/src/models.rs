// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The models that are downloaded rather than compiled in: what they are
//! called, where they come from, and where they live once fetched.
//!
//! Each is one ONNX file in `<app data>/cutout-models/`, named as the
//! table says; a file that is there and whole is an installed model, and
//! a fetch lands in a `.part` beside it and renames at the end, so a
//! killed download leaves nothing that could be mistaken for one. The
//! fetching itself is the host's, since it is the host that has a network
//! and a job slot; this crate only knows the table.
//!
//! A model is fetched from Concat's own mirror, and `upstream` is where
//! that mirror was filled from and the second thing tried. The table is
//! the engine's half of `models/manifest.toml`, which is what fills the
//! mirror; `scripts/models.py --check` keeps the two saying the same
//! thing.

use std::path::{Path, PathBuf};

/// The downloadable models.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ModelId {
    /// Robust Video Matting on MobileNetV3: a person, with a true alpha
    /// edge, carrying state from one frame to the next.
    Person,
    /// IS-Net from the DIS paper: the one thing a picture is of, whatever
    /// it is.
    Object,
    /// SlimSAM's image encoder: a picture into the embeddings the brush
    /// decoder reads.
    BrushEncoder,
    /// SlimSAM's prompt encoder and mask decoder: points on the picture
    /// into the mask of what is under them.
    BrushDecoder,
    /// Real-ESRGAN's compact general model: a picture four times its size
    /// with its noise and compression gone, what Enhance is made of.
    Enhance,
}

/// One model as the table describes it.
#[derive(Clone, Copy, Debug)]
pub struct ModelSpec {
    /// Which model.
    pub id: ModelId,
    /// The file name on disk, what a mask store records, and what the
    /// file is called on the mirror.
    pub file: &'static str,
    /// Where the mirror was filled from, and the second place a download
    /// tries; the host's `models` module is what puts the two in order.
    pub upstream: &'static str,
    /// Its size, for a progress bar before the server says.
    pub bytes: u64,
    /// What a finished download must hash to; a download with nothing to
    /// check against is refused.
    pub sha256: &'static str,
    /// The licence it comes under, for the notices.
    pub licence: &'static str,
}

/// Every downloadable model.
pub const MODELS: [ModelSpec; 5] = [
    ModelSpec {
        id: ModelId::Person,
        file: "rvm-mobilenetv3.onnx",
        upstream: "https://github.com/PeterL1n/RobustVideoMatting/releases/download/v1.0.0/rvm_mobilenetv3_fp32.onnx",
        bytes: 14_975_696,
        sha256: "88d4531297118f595bf2fd60f6f566aec2e559393802d1f436c380f0cbbd2828",
        licence: "GPL-3.0",
    },
    ModelSpec {
        id: ModelId::Object,
        file: "isnet-general-use.onnx",
        upstream: "https://github.com/danielgatis/rembg/releases/download/v0.0.0/isnet-general-use.onnx",
        bytes: 178_648_008,
        sha256: "60920e99c45464f2ba57bee2ad08c919a52bbf852739e96947fbb4358c0d964a",
        licence: "Apache-2.0",
    },
    ModelSpec {
        id: ModelId::BrushEncoder,
        file: "slimsam-77-encoder.onnx",
        upstream: "https://huggingface.co/Xenova/slimsam-77-uniform/resolve/5850ab45f587c112167512ffef949107115e26a0/onnx/vision_encoder.onnx",
        bytes: 23_276_014,
        sha256: "9f8433273a6750b587779baa0cf5508111001bf7e7acfcf585d370139fd366d0",
        licence: "Apache-2.0",
    },
    ModelSpec {
        id: ModelId::BrushDecoder,
        file: "slimsam-77-decoder.onnx",
        upstream: "https://huggingface.co/Xenova/slimsam-77-uniform/resolve/5850ab45f587c112167512ffef949107115e26a0/onnx/prompt_encoder_mask_decoder.onnx",
        bytes: 16_557_892,
        sha256: "f4514391764fbd56e08e119060d874ecd7d52994bfb1968af159e12d4943b5bb",
        licence: "Apache-2.0",
    },
    ModelSpec {
        id: ModelId::Enhance,
        file: "realesr-general-x4v3.onnx",
        upstream: "https://huggingface.co/CoderViking/realesr-general-x4v3-onnx/resolve/c6a971706797c7502945a2b4c4274fce4900d4ab/realesr-general-x4v3.onnx",
        bytes: 4_866_417,
        sha256: "1940a93ee08283a0a7286183186357b1688fe9fa8ede74604b424586aaddf112",
        licence: "BSD-3-Clause",
    },
];

impl ModelId {
    /// The table's row for this model.
    pub fn spec(self) -> &'static ModelSpec {
        MODELS
            .iter()
            .find(|spec| spec.id == self)
            .expect("every model is in the table")
    }

    /// The name a mask store records for masks this model made.
    pub fn name(self) -> &'static str {
        match self {
            ModelId::Person => "rvm-mobilenetv3",
            ModelId::Object => "isnet-1024",
            ModelId::BrushEncoder => "slimsam-77-encoder",
            ModelId::BrushDecoder => "slimsam-77-decoder",
            ModelId::Enhance => "realesr-general-x4v3",
        }
    }
}

/// Where the models live: `<app data>/cutout-models/`.
pub fn models_dir(data: &Path) -> PathBuf {
    data.join("cutout-models")
}

/// The file a model is, once fetched.
pub fn model_file(data: &Path, id: ModelId) -> PathBuf {
    models_dir(data).join(id.spec().file)
}

/// Whether a model is on disk and whole.
pub fn installed(data: &Path, id: ModelId) -> bool {
    std::fs::metadata(model_file(data, id)).is_ok_and(|meta| meta.is_file() && meta.len() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_model_has_its_own_file_and_name() {
        let files: std::collections::HashSet<_> = MODELS.iter().map(|m| m.file).collect();
        assert_eq!(files.len(), MODELS.len());
        let names: std::collections::HashSet<_> = MODELS.iter().map(|m| m.id.name()).collect();
        assert_eq!(names.len(), MODELS.len());
        assert!(!installed(Path::new("/nowhere"), ModelId::Person));
    }
}
