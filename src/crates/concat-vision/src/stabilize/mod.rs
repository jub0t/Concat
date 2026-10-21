// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Taking the camera's shake out of a picture.
//!
//! Done the way Premiere's Warp Stabilizer and Concat's own cutout are:
//! analysed once, cached, applied live. In four parts:
//!
//! - [`track`]: the camera's movement between two frames, measured. Good
//!   corners are found spread over a grid and followed to the next frame
//!   by pyramidal Lucas-Kanade optical flow, each track checked by
//!   following it back again.
//! - [`fit`]: the movement as one similarity - a shift, a turn and a
//!   zoom - fitted to the tracks by RANSAC, so a subject moving across
//!   the frame is outvoted by the background rather than dragging the
//!   answer with it.
//! - [`store`]: the measured movement of every frame of a media file,
//!   cached in the project folder and keyed by the file and the
//!   analysis's version. The host's job writes it; the renderer reads it.
//! - [`path`]: the correction, from the cached movement and the clip's
//!   settings: the camera's path accumulated, smoothed, the difference
//!   taken, and the one zoom the whole clip needs to keep its edges out
//!   of view. Pure and fast, so it is worked out again whenever a setting
//!   changes rather than cached.
//!
//! Every position here is in the picture's own normalised units: the
//! centre of the frame is the origin, `y` runs down, and the frame's
//! height is one, so a turn is a turn whatever the picture's aspect and
//! whatever size it was analysed or decoded at.

pub mod fit;
pub mod path;
pub mod store;
pub mod track;

pub use fit::Similarity;
pub use path::{Correction, Settings};
pub use store::{FrameMotion, Motion, Step, motion_dir};

/// Bumped whenever the analysis would measure differently, so a cache
/// from an older one is measured again rather than trusted.
pub const ANALYSIS_VERSION: u32 = 1;

/// The longest side the analysis decodes at. Shake is a few pixels of a
/// full frame; at this size it is still a pixel or more, and a frame
/// costs milliseconds rather than tens of them.
pub const ANALYSIS_SIDE: u32 = 480;
