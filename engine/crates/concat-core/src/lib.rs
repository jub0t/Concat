// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Core vocabulary of the Concat engine.
//!
//! This crate holds the types that every other crate needs to agree on, and
//! nothing else. It has no dependencies and does no IO.
//!
//! - [`time`] - exact rational timestamps. Every time value in Concat is a
//!   [`time::Rational`] number of seconds. Never `f64`.
//! - [`arena`] - generational arenas and `Copy` handles, the way Concat models
//!   graphs.
//! - [`frame`] - a decoded RGBA8 image buffer.
//! - [`timeline`] - projects, tracks and clips.
//!
//! If you are about to add a dependency to this crate, the thing you are adding
//! probably belongs in `concat-media` or `concat-render` instead.

pub mod arena;
pub mod frame;
pub mod retime;
pub mod time;
pub mod timeline;

pub use frame::Frame;
pub use retime::SpeedCurve;
pub use time::{FrameRate, Rational, TimeRange};
pub use timeline::{Clip, ClipId, Project, Timeline, Track, TrackId, TrackKind};
