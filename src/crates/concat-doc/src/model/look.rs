// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! What titles and shapes are: the two items that draw their own
//! picture. And the look a caption lane gives the titles on it.
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

/// How the captions on a caption lane look and where they sit: the one
/// look that restyling the lane gives every caption on it.
///
/// The captions themselves stay ordinary titles on the lane, each with
/// the whole of its look, so whatever draws a title draws a caption, and
/// a caption restyled on its own is a title like any other. The lane's
/// look is what the next caption placed there takes, and what "apply to
/// all" puts back on every one.
#[derive(Clone, PartialEq, Debug)]
pub struct CaptionLook {
    /// The look, as a title holds it. It has no words and no runs of its
    /// own: each caption's words, and any of them styled apart, are the
    /// caption's.
    pub title: Title,
    /// Down from centred, as a share of the frame's height: where a
    /// caption's `picture.offset_y` is put.
    pub offset_y: f64,
}

impl CaptionLook {
    /// A caption of these words in this look: the look's title, with the
    /// words and the stretches of them styled apart.
    pub fn title_for(&self, words: impl Into<String>, runs: Vec<TextRun>) -> Title {
        Title {
            content: words.into(),
            runs,
            ..self.title.clone()
        }
    }
}

impl Default for CaptionLook {
    /// A title's own look, with no words, centred.
    fn default() -> CaptionLook {
        CaptionLook {
            title: Title {
                content: String::new(),
                ..Title::default()
            },
            offset_y: 0.0,
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

impl Title {
    /// The words as they are painted, in the title's case, and the runs
    /// moved onto the characters their own became: a case can turn one
    /// character into two (a German sharp s, upper-cased, is "SS"), so a
    /// run's ends count characters of what is painted, not of what was
    /// typed. A case this build does not know paints the words as typed.
    pub fn display(&self) -> (String, Vec<TextRun>) {
        let mut out = String::with_capacity(self.content.len());
        // Where each typed character starts in what is painted, and one
        // past the last.
        let mut starts: Vec<u32> = Vec::with_capacity(self.content.len() + 1);
        let mut count: u32 = 0;
        let mut sentence_start = true;
        for c in self.content.chars() {
            starts.push(count);
            let mut push = |s: &mut String, piece: &mut dyn Iterator<Item = char>| {
                for p in piece {
                    s.push(p);
                    count += 1;
                }
            };
            match self.case {
                TextCase::Upper => push(&mut out, &mut c.to_uppercase()),
                TextCase::Lower => push(&mut out, &mut c.to_lowercase()),
                TextCase::Sentence => {
                    if sentence_start && c.is_alphabetic() {
                        push(&mut out, &mut c.to_uppercase());
                        sentence_start = false;
                    } else {
                        push(&mut out, &mut c.to_lowercase());
                        if c.is_alphanumeric() {
                            sentence_start = false;
                        }
                    }
                    if matches!(c, '.' | '!' | '?' | '\n') {
                        sentence_start = true;
                    }
                }
                TextCase::Original | TextCase::Unknown(_) => {
                    push(&mut out, &mut std::iter::once(c))
                }
            }
        }
        starts.push(count);
        let at = |i: u32| starts.get(i as usize).copied().unwrap_or(count);
        let runs = self
            .runs
            .iter()
            .map(|run| TextRun {
                start: at(run.start),
                end: at(run.end),
                ..run.clone()
            })
            .filter(|run| run.end > run.start)
            .collect();
        (out, runs)
    }
}

impl ShapeKind {
    /// The figure's height over its width when it is given no height of
    /// its own: one for the figures drawn in a square, less for the ones
    /// that are wider than tall. A line has no height but its stroke's.
    pub fn natural_height(&self) -> f64 {
        match self {
            ShapeKind::Triangle => 0.866,
            ShapeKind::Parallelogram | ShapeKind::Trapezoid => 0.6,
            ShapeKind::Line | ShapeKind::Arrow => 0.0,
            ShapeKind::Square
            | ShapeKind::Circle
            | ShapeKind::Polygon
            | ShapeKind::Star
            | ShapeKind::Heart
            | ShapeKind::Unknown(_) => 1.0,
        }
    }

    /// Whether the figure is a line, with ends that wear heads.
    pub fn is_line(&self) -> bool {
        matches!(self, ShapeKind::Line | ShapeKind::Arrow)
    }
}

impl ShapeStyle {
    /// The figure's box, width and height as fractions of the frame's
    /// height: the height its own, or the kind's natural proportion of
    /// the width when it is zero. A line's height is its stroke's width.
    pub fn figure_box(&self) -> (f64, f64) {
        let width = self.width.max(0.0);
        let height = if self.kind.is_line() {
            self.stroke_width.max(0.0)
        } else if self.height > 0.0 {
            self.height
        } else {
            width * self.kind.natural_height()
        };
        (width, height)
    }

    /// What a line's start wears: its own, or nothing.
    pub fn start(&self) -> LineHead {
        self.start_head.clone().unwrap_or(LineHead::None)
    }

    /// What a line's end wears: its own, or the kind's - an arrowhead on
    /// an arrow, nothing on a line.
    pub fn end(&self) -> LineHead {
        match (&self.end_head, &self.kind) {
            (Some(head), _) => head.clone(),
            (None, ShapeKind::Arrow) => LineHead::Arrow,
            (None, _) => LineHead::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_paints_in_its_case_and_its_runs_follow_the_characters() {
        let title = Title {
            content: "straße. hello".to_owned(),
            case: TextCase::Upper,
            runs: vec![TextRun {
                start: 8,
                end: 13,
                italic: Some(true),
                ..TextRun::default()
            }],
            ..Title::default()
        };
        let (words, runs) = title.display();
        assert_eq!(words, "STRASSE. HELLO");
        assert_eq!((runs[0].start, runs[0].end), (9, 14));
        let sentence = Title {
            content: "one. TWO! three".to_owned(),
            case: TextCase::Sentence,
            ..Title::default()
        };
        assert_eq!(sentence.display().0, "One. Two! Three");
    }

    #[test]
    fn a_shape_box_takes_its_kind_s_proportion_and_an_arrow_its_head() {
        let square = ShapeStyle::default();
        assert_eq!(square.figure_box(), (0.35, 0.35));
        let tall = ShapeStyle {
            height: 0.5,
            ..ShapeStyle::default()
        };
        assert_eq!(tall.figure_box(), (0.35, 0.5));
        let arrow = ShapeStyle {
            kind: ShapeKind::Arrow,
            ..ShapeStyle::default()
        };
        assert_eq!(arrow.end(), LineHead::Arrow);
        assert_eq!(arrow.start(), LineHead::None);
        assert_eq!(arrow.figure_box().1, arrow.stroke_width);
    }
}
