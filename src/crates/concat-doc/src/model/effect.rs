// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Effects in a chain, and an item's masks.

use std::collections::BTreeMap;

use super::animated::Animated;
use super::ids::{EffectId, MaskId, Raw, Residual};

/// One effect package as an item uses it: in a chain of picture effects
/// or of sound filters, as the package of a transition, or on the cut
/// into an item.
///
/// The package is named by the id its manifest gives (`"concat.mono"`),
/// not minted and not checked against a catalogue: an effect this build
/// does not have still loads, shows as missing, and is written back.
#[derive(Clone, PartialEq, Debug)]
pub struct Effect {
    /// Its own id, from the document's count: what a command names it
    /// by. No other effect, mask or marker of the document has it.
    pub id: EffectId,
    /// Which package. Never empty.
    pub package: String,
    /// Off keeps its settings and does nothing.
    pub enabled: bool,
    /// The knobs that were set, by the key the package's manifest gives
    /// them. One left out is at the package's default.
    pub params: BTreeMap<String, Param>,
    /// Where it works: only inside this mask of the same item, or, with
    /// none, over the whole picture.
    pub mask: Option<MaskId>,
    /// Fields this build does not know.
    pub residual: Residual,
}

impl Effect {
    /// A package at its defaults, switched on, over the whole picture.
    pub fn new(id: EffectId, package: impl Into<String>) -> Effect {
        Effect {
            id,
            package: package.into(),
            enabled: true,
            params: BTreeMap::new(),
            mask: None,
            residual: Residual::default(),
        }
    }
}

/// One knob of an effect: a value it was set to, keys, or both.
///
/// With keys and no value, taking the keys off leaves the knob at the
/// package's default. With neither it would be at its default anyway,
/// and is left out of the map instead: that is the one way to say so.
pub type Param = Animated<f64, Option<f64>>;

/// One of an item's masks: a shape that keeps part of the picture, takes
/// part away, or waits for an effect to use it.
///
/// It is drawn over the picture as the item has it after the crop and
/// the flips, before the placement, so it moves with the item. Its
/// numbers are its own and so are their keys: a mask taken to another
/// item takes its animation with it.
#[derive(Clone, PartialEq, Debug)]
pub struct Mask {
    /// Its own id, from the document's count. No other effect, mask or
    /// marker of the document has it.
    pub id: MaskId,
    /// What the panel calls it; empty for the panel's own numbering.
    pub name: String,
    /// How it joins the masks above it in the item's list.
    pub mode: MaskMode,
    /// Keeps the outside of the shape instead of the inside.
    pub invert: bool,
    /// What it is drawn from.
    pub source: MaskSource,
    /// Its centre's offset from the picture's, as a share of the
    /// picture's width; right is above zero.
    pub x: Animated<f64>,
    /// The same down the picture's height; down is above zero.
    pub y: Animated<f64>,
    /// A factor on its size, both ways; above zero.
    pub scale: Animated<f64>,
    /// Clockwise degrees about its centre.
    pub rotation: Animated<f64>,
    /// How soft its edge is, from 0 to 1.
    pub feather: Animated<f64>,
    /// How far its edge is moved out, in the picture's shorter side; in
    /// when below zero.
    pub expand: Animated<f64>,
    /// How much it counts, from 0 to 1.
    pub opacity: Animated<f64>,
    /// Fields this build does not know.
    pub residual: Residual,
}

/// What a mask is drawn from.
#[derive(Clone, PartialEq, Debug)]
pub enum MaskSource {
    /// A shape in a box.
    Shape {
        /// Which shape.
        shape: MaskShape,
        /// The box's width in the picture's shorter side, before the
        /// mask's scale; not below zero.
        width: Animated<f64>,
        /// Its height on the same terms; a band's thickness.
        height: Animated<f64>,
        /// How round a box's corners are, from 0 to 1.
        roundness: Animated<f64>,
    },
    /// A source this build does not know.
    Unknown(Raw),
}

/// How a mask joins the masks above it.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum MaskMode {
    /// Keeps what it covers as well.
    Add,
    /// Takes away what it covers.
    Subtract,
    /// Keeps only what it and those above share.
    Intersect,
    /// Cuts nothing: kept for an effect to use.
    None,
    /// A mode this build does not know.
    Unknown(Raw),
}

/// Which shape a mask keeps.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum MaskShape {
    /// One side of a line through the centre.
    Linear,
    /// A band along a line through the centre.
    Mirror,
    /// An ellipse.
    Circle,
    /// A box, its corners rounded by the roundness.
    Rectangle,
    /// A heart, point down.
    Heart,
    /// A five-pointed star, point up.
    Star,
    /// A shape this build does not know.
    Unknown(Raw),
}

impl Mask {
    /// A circle of two thirds of the picture's shorter side in the
    /// middle, adding, at full strength, with a hard edge.
    pub fn circle(id: MaskId) -> Mask {
        Mask {
            id,
            name: String::new(),
            mode: MaskMode::Add,
            invert: false,
            source: MaskSource::Shape {
                shape: MaskShape::Circle,
                width: Animated::fixed(0.66),
                height: Animated::fixed(0.66),
                roundness: Animated::fixed(0.0),
            },
            x: Animated::fixed(0.0),
            y: Animated::fixed(0.0),
            scale: Animated::fixed(1.0),
            rotation: Animated::fixed(0.0),
            feather: Animated::fixed(0.0),
            expand: Animated::fixed(0.0),
            opacity: Animated::fixed(1.0),
            residual: Residual::default(),
        }
    }
}
