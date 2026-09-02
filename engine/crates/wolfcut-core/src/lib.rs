// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Core vocabulary of the WolfCut engine.
//!
//! This crate holds the types that every other crate needs to agree on, and
//! nothing else. It has no dependencies and does no IO.
//!
//! - [`time`] - exact rational timestamps. Every time value in WolfCut is a
//!   [`time::Rational`] number of seconds. Never `f64`.
//! - [`arena`] - generational arenas and `Copy` handles, the way WolfCut models
//!   graphs. See `docs/decisions/0003-arena-handles-not-pointers.md`.
//! - [`frame`] - a decoded RGBA8 image buffer.
//! - [`timeline`] - projects, tracks and clips.
//!
//! If you are about to add a dependency to this crate, the thing you are adding
//! probably belongs in `wolfcut-media` or `wolfcut-render` instead.

pub mod arena;
pub mod frame;
pub mod time;
pub mod timeline;

pub use frame::Frame;
pub use time::{FrameRate, Rational, TimeRange};
pub use timeline::{Clip, ClipId, Project, Timeline, Track, TrackId, TrackKind};
