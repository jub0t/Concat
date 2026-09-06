// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! What the engine sees in a picture.
//!
//! A cutout takes a picture's background away without a key colour: a
//! model says, pixel by pixel, how likely each one is to be the person, and
//! the picture's alpha is multiplied by the answer. This crate is the whole
//! of that, in four parts:
//!
//! - [`mask`]: the answer itself, an eight-bit probability picture, with
//!   the sampling, softening and PNG form everything else uses.
//! - [`strokes`]: the corrections. A custom cutout paints brush strokes
//!   over the model's mask; this is where a stroke becomes pixels. The
//!   smart brushes name a thing rather than paint a disc: [`brush`] reads
//!   the thing under a stroke with a small Segment Anything, and the
//!   region it answers is kept beside the masks.
//! - [`apply`]: the frame with its background gone. One function, called
//!   by the exporter and the monitor alike, so the file and the screen
//!   agree by construction.
//! - [`store`]: where masks live between runs. They are keyed by the media
//!   file and the source instant, and cached in the project folder like
//!   its waveforms, so a cutout is found once and travels with the edit.
//!
//! Finding the mask is [`segment`], behind the `infer` feature: three
//! models run by ONNX Runtime through [`runtime`], on the platform's
//! accelerator where there is one. A person matting model and a general
//! object model are downloaded on first use ([`models`] says where from);
//! a small person model is compiled in so a cutout works with nothing
//! downloaded at all. The renderer reads masks and never infers; the
//! host infers and writes them.
//!
//! A mask is whatever shape its model answers in - square for the object
//! model, the picture's own aspect for the person model - and every
//! position in one is a fraction of the source picture: `(0, 0)` its
//! top-left, `(1, 1)` its bottom-right. Strokes are stored in the same
//! fractions. A crop, a flip or a change of output size therefore changes
//! nothing about a mask - the mapping from a decoded pixel back to a
//! source fraction is [`apply::Mapping`], and it is the one place those
//! are undone.

pub mod apply;
#[cfg(feature = "infer")]
pub mod brush;
pub mod mask;
pub mod models;
#[cfg(feature = "infer")]
pub mod runtime;
#[cfg(feature = "infer")]
pub mod segment;
pub mod store;
pub mod strokes;

pub use apply::{Mapping, cut};
#[cfg(feature = "infer")]
pub use brush::{Brush, Embedding};
pub use mask::Mask;
pub use models::ModelId;
#[cfg(feature = "infer")]
pub use segment::Segmenter;
pub use store::{MaskStore, mask_dir, region_file};

/// Masks are found this many times a second of source. Ten is where a
/// person's outline stops visibly lagging their movement, and where a
/// minute of footage is six hundred inferences rather than eighteen hundred.
pub const MASK_RATE: u32 = 10;

/// The compiled-in person model's input and output edge, in pixels.
pub const MODEL_SIZE: u32 = 256;

/// Names the compiled-in model. A mask store records which model made
/// its masks, so a different model never serves another's.
pub const MODEL_ID: &str = "selfie-256";
