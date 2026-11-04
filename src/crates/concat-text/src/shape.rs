// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A shape clip painted: a figure and its paint on a transparent canvas the
//! frame's size, for the compositor to place as it places a title (see the
//! crate docs). The figure is centred on the canvas, so the clip's offset
//! and rotation mean what they mean for a title, and the block is the
//! figure's own box, for a monitor to outline.
//!
//! Everything a person sets on a shape is drawn here in one pass: the
//! figure's path, corners rounded along it; the inside, one colour or a
//! blend; the outline, straddling the edge, inside it or outside it,
//! solid, dashed or dotted, and trimmed to part of the way round; a line's
//! ends; and a shadow or glow under the lot.

use tiny_skia::{
    Color, FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Mask, Paint, Path,
    PathBuilder, PathSegment, Pixmap, PixmapPaint, Point, RadialGradient, Rect,
    SpreadMode, Stroke, StrokeDash, Transform,
};

use crate::{Error, Rendered, RenderedFrame, colour, straight_rgba};

/// The figures a shape clip can be. Mirrors the document's own list; this
/// crate does not depend on the document.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Figure {
    /// A rectangle.
    Square,
    /// An ellipse.
    Circle,
    /// Point up, its base the box's bottom.
    Triangle,
    /// A slanted rectangle, leaning right.
    Parallelogram,
    /// A rectangle narrower at the top.
    Trapezoid,
    /// A regular polygon of this many sides, a point up.
    Polygon(u32),
    /// A star of this many points, its inner corners this far out.
    Star(u32, f32),
    /// A heart.
    Heart,
    /// A straight stroke across the box.
    Line,
}

/// What a line wears on an end.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Head {
    /// The line's own round end.
    #[default]
    None,
    /// Two strokes, open.
    Arrow,
    /// A filled point.
    Triangle,
    /// A filled dot.
    Circle,
    /// A short bar across.
    Bar,
}

/// Where the outline sits on the edge.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Align {
    /// Half in, half out.
    #[default]
    Center,
    /// All inside.
    Inside,
    /// All outside.
    Outside,
}

/// The outline's pattern.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Dash {
    /// Unbroken.
    #[default]
    Solid,
    /// Dashes.
    Dashed,
    /// Round dots.
    Dotted,
}

/// A blend from the fill into `color`.
#[derive(Clone, PartialEq, Debug)]
pub struct Gradient {
    /// Out from the middle, rather than straight across.
    pub radial: bool,
    /// Where the blend ends, `#rrggbb[aa]`.
    pub color: String,
    /// A straight blend's direction in degrees: 0 left to right, 90 down.
    pub angle: f64,
}

/// A shadow, or a glow when it has no distance.
#[derive(Clone, PartialEq, Debug)]
pub struct Shadow {
    /// `#rrggbb[aa]`.
    pub color: String,
    /// Softness, a fraction of frame height.
    pub blur: f64,
    /// Offset, a fraction of frame height.
    pub distance: f64,
    /// Direction in degrees: 90 straight down.
    pub angle: f64,
}

/// Everything about a shape's look, in the document's terms: sizes are
/// fractions of the frame's height, and the box is already resolved -
/// a height the document leaves to the figure arrives here as a number.
#[derive(Clone, PartialEq, Debug)]
pub struct ShapeStyle {
    /// Which figure.
    pub figure: Figure,
    /// The figure's box: width, or a line's length, and height, which for
    /// a line is the room its heads take.
    pub width: f64,
    /// See `width`.
    pub height: f64,
    /// Corners, 0 sharp to 1 as round as the shorter side allows.
    pub corner_radius: f64,
    /// The inside's colour, `#rrggbb[aa]`; empty for an outline alone.
    pub fill: String,
    /// The inside blended into a second colour.
    pub gradient: Option<Gradient>,
    /// The outline's colour; a line is drawn in this, falling back to
    /// `fill` when it is empty.
    pub stroke: String,
    /// The outline's thickness, and a line's own.
    pub stroke_width: f64,
    /// Where the outline sits.
    pub align: Align,
    /// The outline's pattern.
    pub dash: Dash,
    /// A line's ends.
    pub start_head: Head,
    /// See `start_head`.
    pub end_head: Head,
    /// Under the shape.
    pub shadow: Option<Shadow>,
    /// The outline from this far round to that far, 0 to 1.
    pub trim_start: f64,
    /// See `trim_start`.
    pub trim_end: f64,
}

impl Default for ShapeStyle {
    fn default() -> ShapeStyle {
        ShapeStyle {
            figure: Figure::Square,
            width: 0.35,
            height: 0.35,
            corner_radius: 0.0,
            fill: "#ffffff".to_owned(),
            gradient: None,
            stroke: String::new(),
            stroke_width: 0.0,
            align: Align::Center,
            dash: Dash::Solid,
            start_head: Head::None,
            end_head: Head::None,
            shadow: None,
            trim_start: 0.0,
            trim_end: 1.0,
        }
    }
}

/// Paints `style` centred on a `width` × `height` transparent canvas and
/// returns it PNG-encoded with the figure's box. The counterpart of
/// [`crate::render`] for a shape.
pub fn render_shape(style: &ShapeStyle, width: u32, height: u32) -> Result<Rendered, Error> {
    let (canvas, block) = paint_shape(style, width, height)?;
    let png = canvas
        .encode_png()
        .map_err(|error| Error::Encode(error.to_string()))?;
    Ok(Rendered {
        png,
        width,
        height,
        overscan: 1.0,
        block_width: block.0,
        block_height: block.1,
        block_dx: 0,
        block_dy: 0,
        words: Vec::new(),
    })
}

/// [`render_shape`], but the pixels rather than a PNG of them; the
/// counterpart of [`crate::render_frame`].
pub fn render_shape_frame(
    style: &ShapeStyle,
    width: u32,
    height: u32,
) -> Result<RenderedFrame, Error> {
    let (canvas, block) = paint_shape(style, width, height)?;
    Ok(RenderedFrame {
        rgba: straight_rgba(&canvas),
        width,
        height,
        overscan: 1.0,
        block_width: block.0,
        block_height: block.1,
        block_dx: 0,
        block_dy: 0,
        words: Vec::new(),
    })
}

/// The canvas with the shape on it, and the box the monitor outlines.
fn paint_shape(style: &ShapeStyle, width: u32, height: u32) -> Result<(Pixmap, (u32, u32)), Error> {
    let mut canvas = Pixmap::new(width, height).ok_or(Error::Canvas(width, height))?;
    let frame_h = height as f32;
    // The outline's thickness in pixels, never under a pixel once it is
    // asked for at all, so a hairline at 480p is still a line.
    let rule = {
        let asked = (style.stroke_width.max(0.0) as f32) * frame_h;
        if asked > 0.0 { asked.max(1.0) } else { 0.0 }
    };
    let w = (style.width.clamp(0.01, 4.0) as f32) * frame_h;
    let h = if style.figure == Figure::Line {
        (style.height.max(0.0) as f32 * frame_h).max(rule)
    } else {
        (style.height.clamp(0.005, 4.0) as f32) * frame_h
    };
    let (cx, cy) = (width as f32 / 2.0, frame_h / 2.0);
    let bounds = Rect::from_xywh(cx - w / 2.0, cy - h / 2.0, w, h).ok_or(Error::Canvas(width, height))?;

    // Everything is drawn onto a layer first, so a shadow can be cast from
    // exactly what was drawn.
    let mut layer = Pixmap::new(width, height).ok_or(Error::Canvas(width, height))?;
    let edge = colour(&style.stroke);
    let fill = colour(&style.fill);
    let trimmed = style.trim_start > 0.0 || style.trim_end < 1.0;

    if style.figure == Figure::Line {
        if let Some(ink) = edge.or(fill).filter(|_| rule > 0.0) {
            paint_line(&mut layer, style, bounds, rule, ink);
        }
    } else if let Some(path) = figure_path(style, bounds) {
        if let Some(fill) = fill {
            let paint = fill_paint(style, fill, bounds);
            layer.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
        if rule > 0.0
            && let Some(edge) = edge
        {
            let outline = if trimmed {
                trim(&path, style.trim_start as f32, style.trim_end as f32)
            } else {
                Some(path.clone())
            };
            if let Some(outline) = outline {
                let mut stroke = outline_stroke(style.dash, rule);
                // Inside or outside: twice as wide, with the half on the
                // wrong side of the edge masked away.
                let mask = match style.align {
                    Align::Center => None,
                    Align::Inside | Align::Outside => {
                        stroke.width = rule * 2.0;
                        let mut mask = Mask::new(width, height).ok_or(Error::Canvas(width, height))?;
                        mask.fill_path(&path, FillRule::Winding, true, Transform::identity());
                        if style.align == Align::Outside {
                            mask.invert();
                        }
                        Some(mask)
                    }
                };
                if let Some(dash) = dash_of(style.dash, stroke.width) {
                    stroke.dash = Some(dash);
                }
                layer.stroke_path(&outline, &solid(edge), &stroke, Transform::identity(), mask.as_ref());
            }
        }
    }

    if let Some(shadow) = &style.shadow {
        cast_shadow(&mut canvas, &layer, shadow, frame_h);
    }
    canvas.draw_pixmap(0, 0, layer.as_ref(), &PixmapPaint::default(), Transform::identity(), None);

    // The box the monitor outlines: the figure, plus whatever outline sits
    // outside its edge.
    let grown = match (style.figure, edge, style.align) {
        (Figure::Line, _, _) | (_, None, _) => 0.0,
        (_, Some(_), Align::Center) => rule,
        (_, Some(_), Align::Outside) => rule * 2.0,
        (_, Some(_), Align::Inside) => 0.0,
    };
    let grown = if rule > 0.0 { grown } else { 0.0 };
    Ok((
        canvas,
        (
            (w + grown).ceil().max(1.0) as u32,
            (h + grown).ceil().max(1.0) as u32,
        ),
    ))
}

fn solid(colour: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(colour);
    paint.anti_alias = true;
    paint
}

/// The inside's paint: the fill alone, or blended into the gradient's
/// colour across the figure's box.
fn fill_paint(style: &ShapeStyle, fill: Color, bounds: Rect) -> Paint<'static> {
    let Some(gradient) = &style.gradient else {
        return solid(fill);
    };
    let Some(to) = colour(&gradient.color) else {
        return solid(fill);
    };
    let stops = vec![GradientStop::new(0.0, fill), GradientStop::new(1.0, to)];
    let centre = Point::from_xy(bounds.x() + bounds.width() / 2.0, bounds.y() + bounds.height() / 2.0);
    let shader = if gradient.radial {
        let radius = bounds.width().max(bounds.height()) / 2.0;
        RadialGradient::new(centre, 0.0, centre, radius, stops, SpreadMode::Pad, Transform::identity())
    } else {
        // Across the box along the angle, edge to edge: the reach is the
        // box's extent in that direction, so the blend uses all of it.
        let (sin, cos) = (gradient.angle as f32).to_radians().sin_cos();
        let reach = (bounds.width() * cos.abs() + bounds.height() * sin.abs()) / 2.0;
        LinearGradient::new(
            Point::from_xy(centre.x - cos * reach, centre.y - sin * reach),
            Point::from_xy(centre.x + cos * reach, centre.y + sin * reach),
            stops,
            SpreadMode::Pad,
            Transform::identity(),
        )
    };
    match shader {
        Some(shader) => Paint {
            shader,
            anti_alias: true,
            ..Paint::default()
        },
        None => solid(fill),
    }
}

fn outline_stroke(dash: Dash, rule: f32) -> Stroke {
    Stroke {
        width: rule,
        line_cap: if dash == Dash::Dashed { LineCap::Butt } else { LineCap::Round },
        line_join: LineJoin::Round,
        ..Stroke::default()
    }
}

/// A dash pattern for a stroke `width` wide: dashes three widths long with
/// two between, or round dots a width apart (a zero-length dash with a
/// round cap is a dot).
fn dash_of(dash: Dash, width: f32) -> Option<StrokeDash> {
    match dash {
        Dash::Solid => None,
        Dash::Dashed => StrokeDash::new(vec![width * 3.0, width * 2.0], 0.0),
        Dash::Dotted => StrokeDash::new(vec![0.0, width * 2.0], 0.0),
    }
}

/// The figure's closed outline inside `bounds`.
fn figure_path(style: &ShapeStyle, bounds: Rect) -> Option<Path> {
    let (left, top, right, bottom) = (bounds.left(), bounds.top(), bounds.right(), bounds.bottom());
    let (w, h) = (bounds.width(), bounds.height());
    let (cx, cy) = (left + w / 2.0, top + h / 2.0);
    let round = style.corner_radius.clamp(0.0, 1.0) as f32;
    let corners: Vec<Point> = match style.figure {
        // From the top, clockwise, so a trimmed ring - a progress arc -
        // starts at twelve o'clock the way a clock face reads.
        Figure::Circle => {
            let (rx, ry) = (w / 2.0, h / 2.0);
            let k = 0.552_284_8;
            let mut path = PathBuilder::new();
            path.move_to(cx, top);
            path.cubic_to(cx + rx * k, top, right, cy - ry * k, right, cy);
            path.cubic_to(right, cy + ry * k, cx + rx * k, bottom, cx, bottom);
            path.cubic_to(cx - rx * k, bottom, left, cy + ry * k, left, cy);
            path.cubic_to(left, cy - ry * k, cx - rx * k, top, cx, top);
            path.close();
            return path.finish();
        }
        Figure::Heart => return heart(bounds),
        Figure::Line => return None,
        Figure::Square => vec![
            Point::from_xy(left, top),
            Point::from_xy(right, top),
            Point::from_xy(right, bottom),
            Point::from_xy(left, bottom),
        ],
        Figure::Triangle => vec![
            Point::from_xy(cx, top),
            Point::from_xy(right, bottom),
            Point::from_xy(left, bottom),
        ],
        Figure::Parallelogram => {
            let lean = w * 0.2;
            vec![
                Point::from_xy(left + lean, top),
                Point::from_xy(right, top),
                Point::from_xy(right - lean, bottom),
                Point::from_xy(left, bottom),
            ]
        }
        Figure::Trapezoid => {
            let inset = w * 0.2;
            vec![
                Point::from_xy(left + inset, top),
                Point::from_xy(right - inset, top),
                Point::from_xy(right, bottom),
                Point::from_xy(left, bottom),
            ]
        }
        Figure::Polygon(sides) => {
            let sides = sides.clamp(3, 12);
            (0..sides)
                .map(|i| {
                    let turn = -std::f32::consts::FRAC_PI_2
                        + i as f32 * std::f32::consts::TAU / sides as f32;
                    Point::from_xy(cx + turn.cos() * w / 2.0, cy + turn.sin() * h / 2.0)
                })
                .collect()
        }
        Figure::Star(points, inner) => {
            let points = points.clamp(3, 24);
            let inner = inner.clamp(0.05, 0.95);
            (0..points * 2)
                .map(|i| {
                    let turn = -std::f32::consts::FRAC_PI_2
                        + i as f32 * std::f32::consts::PI / points as f32;
                    let reach = if i % 2 == 0 { 1.0 } else { inner };
                    Point::from_xy(
                        cx + turn.cos() * w / 2.0 * reach,
                        cy + turn.sin() * h / 2.0 * reach,
                    )
                })
                .collect()
        }
    };
    rounded_polygon(&corners, round)
}

/// A closed polygon through `corners` with each corner rounded by an arc
/// tangent to both its edges. `round` is how much of the room each corner
/// has - half its shorter edge - the arc takes: 0 sharp, 1 the most there
/// is, which turns a square into a circle and a rectangle into a pill.
fn rounded_polygon(corners: &[Point], round: f32) -> Option<Path> {
    let count = corners.len();
    if count < 3 {
        return None;
    }
    let mut path = PathBuilder::new();
    if round <= 0.0 {
        path.move_to(corners[0].x, corners[0].y);
        for corner in &corners[1..] {
            path.line_to(corner.x, corner.y);
        }
        path.close();
        return path.finish();
    }
    let length = |a: Point, b: Point| ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt();
    let toward = |from: Point, to: Point, by: f32| {
        let d = length(from, to).max(1e-6);
        Point::from_xy(from.x + (to.x - from.x) * by / d, from.y + (to.y - from.y) * by / d)
    };
    for i in 0..count {
        let previous = corners[(i + count - 1) % count];
        let corner = corners[i];
        let next = corners[(i + 1) % count];
        // How far back along each edge the arc starts: half the shorter
        // edge at most, so neighbouring corners never overlap.
        let cut = length(previous, corner).min(length(corner, next)) / 2.0 * round;
        let into = toward(corner, previous, cut);
        let out = toward(corner, next, cut);
        if i == 0 {
            path.move_to(into.x, into.y);
        } else {
            path.line_to(into.x, into.y);
        }
        // A cubic standing in for the arc: its handles two thirds of the
        // way to the corner, which is close to circular for the angles a
        // figure has and exactly what keeps the edge smooth.
        let c1 = toward(into, corner, cut * 0.5523);
        let c2 = toward(out, corner, cut * 0.5523);
        path.cubic_to(c1.x, c1.y, c2.x, c2.y, out.x, out.y);
    }
    path.close();
    path.finish()
}

/// A heart filling `bounds`: two lobes meeting in a notch at the top and
/// curving down to a point at the bottom.
fn heart(bounds: Rect) -> Option<Path> {
    let (x, y, w, h) = (bounds.x(), bounds.y(), bounds.width(), bounds.height());
    let at = |u: f32, v: f32| (x + u * w, y + v * h);
    let mut path = PathBuilder::new();
    let (sx, sy) = at(0.5, 0.25);
    path.move_to(sx, sy);
    let (a, b) = at(0.5, 0.05);
    let (c, d) = at(0.88, -0.02);
    let (e, f) = at(1.0, 0.28);
    path.cubic_to(a, b, c, d, e, f);
    let (a, b) = at(1.0, 0.55);
    let (c, d) = at(0.7, 0.75);
    let (e, f) = at(0.5, 1.0);
    path.cubic_to(a, b, c, d, e, f);
    let (a, b) = at(0.3, 0.75);
    let (c, d) = at(0.0, 0.55);
    let (e, f) = at(0.0, 0.28);
    path.cubic_to(a, b, c, d, e, f);
    let (a, b) = at(0.12, -0.02);
    let (c, d) = at(0.5, 0.05);
    path.cubic_to(a, b, c, d, sx, sy);
    path.close();
    path.finish()
}

/// How long `path` is, its curves measured as short straight pieces.
fn path_length(path: &Path) -> f32 {
    let mut length = 0.0;
    let mut start = Point::zero();
    let mut at = Point::zero();
    let dist = |a: Point, b: Point| ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt();
    for segment in path.segments() {
        match segment {
            PathSegment::MoveTo(p) => {
                start = p;
                at = p;
            }
            PathSegment::LineTo(p) => {
                length += dist(at, p);
                at = p;
            }
            PathSegment::QuadTo(c, p) => {
                let mut last = at;
                for step in 1..=16 {
                    let t = step as f32 / 16.0;
                    let u = 1.0 - t;
                    let q = Point::from_xy(
                        u * u * at.x + 2.0 * u * t * c.x + t * t * p.x,
                        u * u * at.y + 2.0 * u * t * c.y + t * t * p.y,
                    );
                    length += dist(last, q);
                    last = q;
                }
                at = p;
            }
            PathSegment::CubicTo(c1, c2, p) => {
                let mut last = at;
                for step in 1..=16 {
                    let t = step as f32 / 16.0;
                    let u = 1.0 - t;
                    let q = Point::from_xy(
                        u * u * u * at.x + 3.0 * u * u * t * c1.x + 3.0 * u * t * t * c2.x + t * t * t * p.x,
                        u * u * u * at.y + 3.0 * u * u * t * c1.y + 3.0 * u * t * t * c2.y + t * t * t * p.y,
                    );
                    length += dist(last, q);
                    last = q;
                }
                at = p;
            }
            PathSegment::Close => {
                length += dist(at, start);
                at = start;
            }
        }
    }
    length
}

/// The stretch of `path` from `from` to `to` of the way along it: one dash
/// that long, starting that far in.
fn trim(path: &Path, from: f32, to: f32) -> Option<Path> {
    let (from, to) = (from.clamp(0.0, 1.0), to.clamp(0.0, 1.0));
    if to - from <= 1e-4 {
        return None;
    }
    let total = path_length(path);
    if total <= 0.0 {
        return None;
    }
    let shown = (to - from) * total;
    let dash = StrokeDash::new(vec![shown, total * 2.0], -from * total)?;
    path.dash(&dash, 1.0)
}

/// A line across `bounds`, `rule` thick, with its heads, in `ink`.
fn paint_line(layer: &mut Pixmap, style: &ShapeStyle, bounds: Rect, rule: f32, ink: Color) {
    let cy = bounds.y() + bounds.height() / 2.0;
    let arm = (bounds.height() / 2.0).max(rule);
    let head_room = |head: Head| match head {
        Head::None | Head::Bar => rule / 2.0,
        Head::Arrow => rule / 2.0,
        Head::Triangle => arm * 1.2,
        Head::Circle => arm * 0.7,
    };
    let x0 = bounds.left() + head_room(style.start_head);
    let x1 = bounds.right() - head_room(style.end_head);
    let mut shaft = PathBuilder::new();
    shaft.move_to(x0, cy);
    shaft.line_to(x1.max(x0), cy);
    let Some(shaft) = shaft.finish() else {
        return;
    };
    let shaft = if style.trim_start > 0.0 || style.trim_end < 1.0 {
        match trim(&shaft, style.trim_start as f32, style.trim_end as f32) {
            Some(path) => path,
            None => return,
        }
    } else {
        shaft
    };
    let mut stroke = outline_stroke(style.dash, rule);
    stroke.dash = dash_of(style.dash, rule);
    let paint = solid(ink);
    layer.stroke_path(&shaft, &paint, &stroke, Transform::identity(), None);

    // The heads, at the line's true ends; each points away from the line.
    let ends = [
        (style.start_head, bounds.left() + rule / 2.0, -1.0f32),
        (style.end_head, bounds.right() - rule / 2.0, 1.0f32),
    ];
    let plain = Stroke {
        width: rule,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    for (head, tip, out) in ends {
        let mut path = PathBuilder::new();
        match head {
            Head::None => continue,
            Head::Arrow => {
                path.move_to(tip - out * arm, cy - arm);
                path.line_to(tip, cy);
                path.line_to(tip - out * arm, cy + arm);
                if let Some(path) = path.finish() {
                    layer.stroke_path(&path, &paint, &plain, Transform::identity(), None);
                }
            }
            Head::Triangle => {
                let tip = tip + out * rule / 2.0;
                path.move_to(tip, cy);
                path.line_to(tip - out * arm * 1.6, cy - arm);
                path.line_to(tip - out * arm * 1.6, cy + arm);
                path.close();
                if let Some(path) = path.finish() {
                    layer.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
                }
            }
            Head::Circle => {
                path.push_circle(tip - out * (arm * 0.7 - rule / 2.0), cy, arm * 0.7);
                if let Some(path) = path.finish() {
                    layer.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
                }
            }
            Head::Bar => {
                path.move_to(tip, cy - arm);
                path.line_to(tip, cy + arm);
                if let Some(path) = path.finish() {
                    layer.stroke_path(&path, &paint, &plain, Transform::identity(), None);
                }
            }
        }
    }
}

/// `layer`'s silhouette in the shadow's colour, softened and moved, drawn
/// under where the layer will go.
fn cast_shadow(canvas: &mut Pixmap, layer: &Pixmap, shadow: &Shadow, frame_h: f32) {
    let Some(tint) = colour(&shadow.color) else {
        return;
    };
    let (width, height) = (layer.width() as usize, layer.height() as usize);
    let mut alpha: Vec<f32> = layer
        .pixels()
        .iter()
        .map(|pixel| f32::from(pixel.alpha()) / 255.0)
        .collect();
    let radius = (shadow.blur.max(0.0) as f32 * frame_h / 2.0).round() as usize;
    if radius > 0 {
        // Three box blurs come within a few percent of a Gaussian.
        for _ in 0..3 {
            box_blur(&mut alpha, width, height, radius);
        }
    }
    let (sin, cos) = (shadow.angle as f32).to_radians().sin_cos();
    let reach = shadow.distance.max(0.0) as f32 * frame_h;
    let (dx, dy) = ((cos * reach).round() as i64, (sin * reach).round() as i64);
    let strength = tint.alpha();
    let pixels = canvas.pixels_mut();
    for y in 0..height as i64 {
        let sy = y - dy;
        if sy < 0 || sy >= height as i64 {
            continue;
        }
        for x in 0..width as i64 {
            let sx = x - dx;
            if sx < 0 || sx >= width as i64 {
                continue;
            }
            let a = alpha[sy as usize * width + sx as usize] * strength;
            if a <= 0.0 {
                continue;
            }
            let a = a.min(1.0);
            let colour = tiny_skia::ColorU8::from_rgba(
                (tint.red() * 255.0) as u8,
                (tint.green() * 255.0) as u8,
                (tint.blue() * 255.0) as u8,
                (a * 255.0).round() as u8,
            );
            pixels[y as usize * width + x as usize] = colour.premultiply();
        }
    }
}

/// One pass of a box blur `radius` wide over a `width` × `height` plane,
/// across then down, by running sums.
fn box_blur(plane: &mut [f32], width: usize, height: usize, radius: usize) {
    let span = (2 * radius + 1) as f32;
    let mut line = vec![0f32; width.max(height)];
    for y in 0..height {
        let row = &mut plane[y * width..(y + 1) * width];
        let mut sum = 0.0;
        for i in 0..=radius.min(width - 1) {
            sum += row[i];
        }
        sum += row[0] * radius as f32;
        for x in 0..width {
            line[x] = sum / span;
            let add = row[(x + radius + 1).min(width - 1)];
            let drop = row[x.saturating_sub(radius)];
            sum += add - drop;
        }
        row.copy_from_slice(&line[..width]);
    }
    for x in 0..width {
        let at = |y: usize| y * width + x;
        let mut sum = 0.0;
        for i in 0..=radius.min(height - 1) {
            sum += plane[at(i)];
        }
        sum += plane[at(0)] * radius as f32;
        for y in 0..height {
            line[y] = sum / span;
            let add = plane[at((y + radius + 1).min(height - 1))];
            let drop = plane[at(y.saturating_sub(radius))];
            sum += add - drop;
        }
        for y in 0..height {
            plane[at(y)] = line[y];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(frame: &RenderedFrame, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * frame.width + x) * 4) as usize;
        [frame.rgba[at], frame.rgba[at + 1], frame.rgba[at + 2], frame.rgba[at + 3]]
    }

    fn square(fill: &str) -> ShapeStyle {
        ShapeStyle {
            width: 0.5,
            height: 0.5,
            fill: fill.to_owned(),
            ..ShapeStyle::default()
        }
    }

    #[test]
    fn a_shape_is_painted_centred_at_its_size() {
        let frame = render_shape_frame(&square("#ff0000"), 400, 200).expect("paints");
        assert_eq!((frame.block_width, frame.block_height), (100, 100));
        assert_eq!(pixel(&frame, 200, 100), [255, 0, 0, 255], "the centre is red and solid");
        assert_eq!(pixel(&frame, 140, 100)[3], 0, "ten pixels past the edge is clear");

        let line = ShapeStyle {
            figure: Figure::Line,
            width: 0.5,
            height: 0.05,
            fill: "#00ff00".to_owned(),
            stroke_width: 0.05,
            ..ShapeStyle::default()
        };
        let frame = render_shape_frame(&line, 400, 200).expect("paints");
        assert_eq!((frame.block_width, frame.block_height), (100, 10));
        assert_eq!(pixel(&frame, 200, 100)[1..], [255, 0, 255][..], "the rule runs through the centre");
        assert_eq!(pixel(&frame, 140, 100)[3], 0, "and stops at its length");

        let png = render_shape(&square("#ff0000"), 64, 64).expect("encodes");
        assert!(png.png.starts_with(b"\x89PNG"));
    }

    #[test]
    fn a_width_and_height_of_its_own_make_a_rectangle() {
        let mut wide = square("#ffffff");
        wide.width = 1.0;
        wide.height = 0.25;
        let frame = render_shape_frame(&wide, 400, 200).expect("paints");
        assert_eq!((frame.block_width, frame.block_height), (200, 50));
        assert_eq!(pixel(&frame, 290, 100)[3], 255, "wide");
        assert_eq!(pixel(&frame, 200, 70)[3], 0, "and short");
    }

    #[test]
    fn rounded_corners_take_the_corner_off_and_leave_the_middle() {
        let mut round = square("#ffffff");
        round.corner_radius = 1.0;
        let frame = render_shape_frame(&round, 400, 200).expect("paints");
        assert_eq!(pixel(&frame, 152, 52)[3], 0, "the corner is gone");
        assert_eq!(pixel(&frame, 200, 52)[3], 255, "the top edge's middle stays");
    }

    #[test]
    fn a_gradient_runs_from_the_fill_into_its_colour() {
        let mut blend = square("#ff0000");
        blend.gradient = Some(Gradient {
            radial: false,
            color: "#0000ff".to_owned(),
            angle: 0.0,
        });
        let frame = render_shape_frame(&blend, 400, 200).expect("paints");
        let left = pixel(&frame, 152, 100);
        let right = pixel(&frame, 248, 100);
        assert!(left[0] > 200 && left[2] < 60, "{left:?}");
        assert!(right[2] > 200 && right[0] < 60, "{right:?}");
    }

    #[test]
    fn an_outline_inside_keeps_the_size_and_outside_adds_to_it() {
        let mut edge = square("");
        edge.stroke = "#ffffff".to_owned();
        edge.stroke_width = 0.05;
        edge.align = Align::Inside;
        let inside = render_shape_frame(&edge, 400, 200).expect("paints");
        assert_eq!(inside.block_width, 100);
        assert_eq!(pixel(&inside, 153, 100)[3], 255, "inside the edge is ink");
        assert_eq!(pixel(&inside, 147, 100)[3], 0, "outside is clear");
        edge.align = Align::Outside;
        let outside = render_shape_frame(&edge, 400, 200).expect("paints");
        assert_eq!(outside.block_width, 120);
        assert_eq!(pixel(&outside, 147, 100)[3], 255, "outside the edge is ink");
        assert_eq!(pixel(&outside, 153, 100)[3], 0, "inside is clear");
    }

    #[test]
    fn a_trimmed_outline_is_drawn_part_of_the_way_round() {
        let mut ring = square("");
        ring.figure = Figure::Circle;
        ring.stroke = "#ffffff".to_owned();
        ring.stroke_width = 0.04;
        ring.trim_start = 0.0;
        ring.trim_end = 0.5;
        let frame = render_shape_frame(&ring, 400, 200).expect("paints");
        let inked = frame.rgba.chunks(4).filter(|p| p[3] > 128).count();
        ring.trim_end = 1.0;
        let whole = render_shape_frame(&ring, 400, 200).expect("paints");
        let all = whole.rgba.chunks(4).filter(|p| p[3] > 128).count();
        let share = inked as f32 / all as f32;
        assert!((0.4..0.6).contains(&share), "half the ring, got {share}");
    }

    #[test]
    fn a_shadow_falls_where_it_is_told() {
        let mut lifted = square("#ffffff");
        lifted.shadow = Some(Shadow {
            color: "#000000".to_owned(),
            blur: 0.0,
            distance: 0.1,
            angle: 90.0,
        });
        let frame = render_shape_frame(&lifted, 400, 200).expect("paints");
        assert_eq!(pixel(&frame, 200, 160), [0, 0, 0, 255], "under the square is its shadow");
        assert_eq!(pixel(&frame, 200, 100), [255, 255, 255, 255], "the square is on top");
        assert_eq!(pixel(&frame, 200, 40)[3], 0, "and nothing above it");
    }

    #[test]
    fn every_figure_paints_something() {
        for figure in [
            Figure::Square,
            Figure::Circle,
            Figure::Triangle,
            Figure::Parallelogram,
            Figure::Trapezoid,
            Figure::Polygon(6),
            Figure::Star(5, 0.45),
            Figure::Heart,
        ] {
            let mut style = square("#ffffff");
            style.figure = figure;
            style.corner_radius = 0.3;
            let frame = render_shape_frame(&style, 200, 200).expect("paints");
            assert_eq!(pixel(&frame, 100, 110)[3], 255, "{figure:?} has a middle");
        }
    }
}

