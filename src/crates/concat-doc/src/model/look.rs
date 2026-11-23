// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! What titles and shapes are: the two items that draw their own
//! picture.
//!
//! Sizes are shares of the frame's height (a title's line length of its
//! width), so what was composed against one frame size lands the same at
//! another.

use super::ids::Raw;
use super::sequence::Color;

/// A title: its words and their look.
#[derive(Clone, PartialEq, Debug)]
pub struct Title {
    /// The words, newlines and all.
    pub content: String,
    /// The family, as CSS names one; may be a font the person added.
    pub font_family: String,
    /// The cap height, as a share of the frame's height; above zero.
    pub font_size: f64,
    /// The weight on CSS's scale, 100 to 900.
    pub font_weight: f64,
    /// Italic.
    pub italic: bool,
    /// Underlined.
    pub underline: bool,
    /// Struck through.
    pub strikethrough: bool,
    /// The fill.
    pub color: Color,
    /// How much of the whole title shows, 0 to 1, over the item's own
    /// opacity.
    pub opacity: f64,
    /// How the lines sit in their block, and which point of it the
    /// item's position pins.
    pub align: TextAlign,
    /// How the words are cased when drawn; the words themselves stay.
    pub case: TextCase,
    /// Baseline to baseline as a multiple of the size; at least 0.5.
    pub line_height: f64,
    /// Extra space between letters, in shares of the frame's height.
    pub tracking: f64,
    /// The longest a line runs before it wraps, as a share of the
    /// frame's width; zero is no limit.
    pub max_width: f64,
    /// The block's height; zero is the words' own.
    pub max_height: f64,
    /// The outline's colour.
    pub stroke_color: Color,
    /// The outline's thickness; zero is none.
    pub stroke_width: f64,
    /// A drop shadow.
    pub shadow: bool,
    /// A plate behind the words, or none.
    pub plate: Option<Color>,
    /// The air between the words and the plate's sides; not below zero.
    pub plate_padding_x: f64,
    /// The same above and below.
    pub plate_padding_y: f64,
    /// The plate's corner radius; not below zero.
    pub plate_radius: f64,
    /// Stretches of the words with a look of their own, in order, apart
    /// and inside the words.
    pub runs: Vec<TextRun>,
}

impl Default for Title {
    /// "Your text" as the window places a title.
    fn default() -> Title {
        Title {
            content: "Your text".to_owned(),
            font_family: "Hanken Grotesk".to_owned(),
            font_size: 0.09,
            font_weight: 700.0,
            italic: false,
            underline: false,
            strikethrough: false,
            color: Color::WHITE,
            opacity: 1.0,
            align: TextAlign::Center,
            case: TextCase::Original,
            line_height: 1.2,
            tracking: 0.0,
            max_width: 0.0,
            max_height: 0.0,
            stroke_color: Color::BLACK,
            stroke_width: 0.0,
            shadow: true,
            plate: None,
            plate_padding_x: 0.0315,
            plate_padding_y: 0.018,
            plate_radius: 0.0135,
            runs: Vec::new(),
        }
    }
}

/// A stretch of a title's words with a look of its own over the
/// title's. `start` and `end` count characters (Unicode scalar values)
/// into the words; `end` is one past the last, and is after `start`.
/// Each field left `None` is the title's.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct TextRun {
    /// The first character.
    pub start: u32,
    /// One past the last.
    pub end: u32,
    /// The fill.
    pub color: Option<Color>,
    /// The size as a multiple of the title's, 0.1 to 10.
    pub font_scale: Option<f64>,
    /// The weight, 100 to 900.
    pub font_weight: Option<f64>,
    /// Italic or upright.
    pub italic: Option<bool>,
    /// Underlined or not.
    pub underline: Option<bool>,
    /// Struck through or not.
    pub strikethrough: Option<bool>,
    /// The outline's colour.
    pub stroke_color: Option<Color>,
}

/// How a title's lines sit in their block.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum TextAlign {
    /// On a shared left edge, where the item's position is.
    Left,
    /// Centred on each other and on the item's position.
    Center,
    /// On a shared right edge.
    Right,
    /// An alignment this build does not know.
    Unknown(Raw),
}

/// How a title's words are cased when drawn.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum TextCase {
    /// All upper case.
    Upper,
    /// All lower case.
    Lower,
    /// Lower case, sentences capitalised.
    Sentence,
    /// As written.
    Original,
    /// A casing this build does not know.
    Unknown(Raw),
}

/// A shape: one figure, how it is painted, and its size.
#[derive(Clone, PartialEq, Debug)]
pub struct ShapeStyle {
    /// The figure.
    pub kind: ShapeKind,
    /// Its width, or a line's length, as a share of the frame's height;
    /// not below zero.
    pub width: f64,
    /// Its height; zero is the figure's own proportion of its width.
    pub height: f64,
    /// How round its corners are, 0 to 1.
    pub corner_radius: f64,
    /// A polygon's sides, 3 to 12.
    pub sides: u32,
    /// A star's points, 3 to 24.
    pub points: u32,
    /// How far out a star's inner corners sit, 0 to 1 of its radius.
    pub inner_radius: f64,
    /// The inside, or none for an outline alone.
    pub fill: Option<Color>,
    /// The inside blended into a second colour, or `fill` alone.
    pub gradient: Option<Gradient>,
    /// The outline's colour, or none; a line is all outline and falls
    /// back to the fill.
    pub stroke: Option<Color>,
    /// The outline's thickness; not below zero.
    pub stroke_width: f64,
    /// Where the outline sits on the edge.
    pub stroke_align: StrokeAlign,
    /// The outline's pattern.
    pub dash: StrokeDash,
    /// The outline drawn from here, 0 to 1 of the way round.
    pub trim_start: f64,
    /// To here.
    pub trim_end: f64,
    /// What a line's start wears; none is the kind's own.
    pub start_head: Option<LineHead>,
    /// What its end wears; none is the kind's own.
    pub end_head: Option<LineHead>,
    /// A shadow or a glow, or none.
    pub shadow: Option<ShapeShadow>,
}

impl Default for ShapeStyle {
    /// A white square, a third of the frame's height across.
    fn default() -> ShapeStyle {
        ShapeStyle {
            kind: ShapeKind::Square,
            width: 0.35,
            height: 0.0,
            corner_radius: 0.0,
            sides: 6,
            points: 5,
            inner_radius: 0.45,
            fill: Some(Color::WHITE),
            gradient: None,
            stroke: None,
            stroke_width: 0.012,
            stroke_align: StrokeAlign::Center,
            dash: StrokeDash::Solid,
            trim_start: 0.0,
            trim_end: 1.0,
            start_head: None,
            end_head: None,
            shadow: None,
        }
    }
}

/// Which figure a shape draws.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ShapeKind {
    /// A rectangle, square until given a height.
    Square,
    /// An ellipse, a disc until given a height.
    Circle,
    /// Point up.
    Triangle,
    /// A rectangle leaning right.
    Parallelogram,
    /// A rectangle narrower at the top.
    Trapezoid,
    /// A straight stroke.
    Line,
    /// A line with an arrowhead on its end.
    Arrow,
    /// A regular polygon, a point up.
    Polygon,
    /// A star.
    Star,
    /// A heart.
    Heart,
    /// A figure this build does not know.
    Unknown(Raw),
}

/// A blend of a shape's fill into a second colour.
#[derive(Clone, PartialEq, Debug)]
pub struct Gradient {
    /// The colour it ends in.
    pub color: Color,
    /// Which way a straight blend runs, in degrees: 0 left to right.
    pub angle: f64,
    /// Out from the middle instead of straight across.
    pub radial: bool,
}

/// A shadow under a shape, or a glow round it.
#[derive(Clone, PartialEq, Debug)]
pub struct ShapeShadow {
    /// Its colour; the alpha is its strength.
    pub color: Color,
    /// Which way it falls, in degrees: 90 is straight down.
    pub angle: f64,
    /// How far from the shape; not below zero.
    pub distance: f64,
    /// How soft; not below zero.
    pub blur: f64,
}

/// Where an outline sits on a figure's edge.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum StrokeAlign {
    /// Half in, half out.
    Center,
    /// All inside.
    Inside,
    /// All outside.
    Outside,
    /// A placing this build does not know.
    Unknown(Raw),
}

/// An outline's pattern.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum StrokeDash {
    /// Unbroken.
    Solid,
    /// Dashes.
    Dashed,
    /// Dots.
    Dotted,
    /// A pattern this build does not know.
    Unknown(Raw),
}

/// What a line wears on an end.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum LineHead {
    /// The line's own round end.
    None,
    /// Two strokes, open.
    Arrow,
    /// A filled point.
    Triangle,
    /// A filled dot.
    Circle,
    /// A short bar across.
    Bar,
    /// A head this build does not know.
    Unknown(Raw),
}
