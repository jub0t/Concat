// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A glyph's colour layers (`COLR`), walked.
//!
//! The font describes the glyph as a tree: outlines that clip what is
//! inside them, fills of a solid colour or a linear, radial or sweep
//! gradient, transforms, and groups composited onto what is under them by
//! a blend mode. The font library walks the tree and calls back here; this
//! module follows it three ways - to collect plain solid layers as paths, to
//! measure where the glyph puts colour, and to paint it into a picture.

use std::sync::Arc;

use tiny_skia::{
    BlendMode, Color, FillRule, GradientStop, LinearGradient, Mask, Paint, Path, PathBuilder,
    Pixmap, PixmapPaint, Point, RadialGradient, Rect, Shader, SpreadMode, SweepGradient, Transform,
};
use ttf_parser::colr::{ClipBox, ColorStop, CompositeMode, GradientExtend, Painter};
use ttf_parser::{GlyphId, RgbaColor};

use super::Picture;
use crate::Outliner;

/// The longest side a glyph's picture may have: past it, the glyph is
/// drawn as its outline.
const MAX_PICTURE_SIDE: u32 = 4096;

/// A glyph's layers, as far as the plain walk can tell.
pub(super) enum Layering {
    /// Nothing but outlines in solid colours, bottom first: each a path in
    /// the line's pixels and its colour. Layers whose glyph has no outline
    /// are left out.
    Solid(Vec<(Path, Color)>),
    /// Clips, transforms, gradients or blends: only a picture will do.
    Rich,
    /// The walk failed - a layer names a colour past the palette's end,
    /// say - and the glyph is not drawn in colour.
    Failed,
}

/// Walks `glyph`'s layers, outlined with the pen at `pen` and `scale`
/// pixels to the font unit, with `foreground` for the text's own colour.
pub(super) fn layers(
    face: &ttf_parser::Face<'_>,
    glyph: GlyphId,
    pen: (f32, f32),
    scale: f32,
    foreground: RgbaColor,
) -> Layering {
    let mut solids = Solids {
        face,
        pen,
        scale,
        held: None,
        layers: Vec::new(),
        rich: false,
    };
    let walked = face.paint_color_glyph(glyph, 0, foreground, &mut solids);
    if solids.rich {
        Layering::Rich
    } else if walked.is_none() {
        Layering::Failed
    } else {
        Layering::Solid(solids.layers)
    }
}

/// Whether `glyph`'s layers walk cleanly and put colour somewhere: what
/// makes a face a colour face for it. The text colour is taken as black.
pub(super) fn shows_anything(face: &ttf_parser::Face<'_>, glyph: GlyphId) -> bool {
    let Some(mut survey) = Walk::new(face, 1.0, (0.0, 0.0), None) else {
        return false;
    };
    face.paint_color_glyph(glyph, 0, RgbaColor::new(0, 0, 0, 255), &mut survey)
        .is_some()
        && survey.ink().is_some()
}

/// `glyph` painted into a picture at an em of `em` pixels, with
/// `foreground` for the text's own colour: measured first, so the picture
/// is just big enough, then painted. `None` where nothing would show.
pub(super) fn paint(
    face: &ttf_parser::Face<'_>,
    glyph: GlyphId,
    em: f32,
    foreground: RgbaColor,
) -> Option<Picture> {
    let scale = em / face.units_per_em() as f32;
    let mut survey = Walk::new(face, scale, (0.0, 0.0), None)?;
    face.paint_color_glyph(glyph, 0, foreground, &mut survey)?;
    let ink = survey.ink()?;
    // A pixel more each way for the antialiasing's fringe.
    let (left, top) = (ink.left().floor() - 1.0, ink.top().floor() - 1.0);
    let width = (ink.right().ceil() + 1.0 - left) as u32;
    let height = (ink.bottom().ceil() + 1.0 - top) as u32;
    if width > MAX_PICTURE_SIDE || height > MAX_PICTURE_SIDE {
        return None;
    }
    let mut walk = Walk::new(face, scale, (left, top), Some(Pixmap::new(width, height)?))?;
    face.paint_color_glyph(glyph, 0, foreground, &mut walk)?;
    let pixels = walk.finish()?;
    if pixels.pixels().iter().all(|pixel| pixel.alpha() == 0) {
        return None;
    }
    Some(Picture {
        pixels: Arc::new(pixels),
        left,
        top,
    })
}

/// The plain walk: solid layers collected as paths, anything else noted.
struct Solids<'f, 'a> {
    face: &'f ttf_parser::Face<'a>,
    pen: (f32, f32),
    scale: f32,
    /// The outline the next fill is for.
    held: Option<Path>,
    layers: Vec<(Path, Color)>,
    rich: bool,
}

impl<'a> Painter<'a> for Solids<'_, 'a> {
    fn outline_glyph(&mut self, glyph: GlyphId) {
        if self.rich {
            return;
        }
        let mut builder = PathBuilder::new();
        let mut outliner = Outliner {
            builder: &mut builder,
            scale: self.scale,
            x: self.pen.0,
            y: self.pen.1,
        };
        let outlined = self.face.outline_glyph(glyph, &mut outliner).is_some();
        self.held = if outlined { builder.finish() } else { None };
    }

    fn paint(&mut self, fill: ttf_parser::colr::Paint<'a>) {
        match fill {
            ttf_parser::colr::Paint::Solid(colour) => {
                if let Some(path) = self.held.take() {
                    self.layers.push((path, colour_of(colour)));
                }
            }
            _ => self.rich = true,
        }
    }

    fn push_clip(&mut self) {
        self.rich = true;
    }

    fn push_clip_box(&mut self, _: ClipBox) {
        self.rich = true;
    }

    fn pop_clip(&mut self) {}

    fn push_layer(&mut self, _: CompositeMode) {
        self.rich = true;
    }

    fn pop_layer(&mut self) {}

    fn push_transform(&mut self, _: ttf_parser::Transform) {
        self.rich = true;
    }

    fn pop_transform(&mut self) {}
}

/// The full walk, measuring or painting.
///
/// Paths are kept in the picture's pixels, already through every
/// transform in force when they were outlined. A fill covers what the
/// clips in force leave open: the innermost clip filled as a path, so its
/// edge is as smooth as a letter's, under a mask of the others. Before any
/// clip or group, as in a plain stack of layers, a fill covers the last
/// outline instead.
struct Walk<'f, 'a> {
    face: &'f ttf_parser::Face<'a>,
    /// Font units to pixels first, then each transform pushed, the one in
    /// force last.
    transforms: Vec<Transform>,
    /// The last outline, or `None` for a glyph that has none.
    held: Option<Path>,
    /// Whether anything has been outlined yet.
    outlined: bool,
    /// Whether a clip or a group has been started.
    nested: bool,
    /// The clips in force, outermost first; `None` is a clip to nothing.
    clips: Vec<Option<Path>>,
    surface: Surface,
}

enum Surface {
    /// Measuring: where colour lands, held within a few ems of the pen.
    Measure { limit: Rect, ink: Option<Rect> },
    /// Painting: the picture, the groups open over it, and whether a group
    /// could not be made, which spoils the picture.
    Draw {
        base: Pixmap,
        groups: Vec<Group>,
        spoilt: bool,
    },
}

/// A group being composited: its own pixels, the blend it goes down with,
/// and how many clips were in force when it began - those clip it as it
/// goes down; the ones pushed inside it clip what is painted into it.
struct Group {
    pixels: Pixmap,
    mode: BlendMode,
    clips: usize,
}

impl<'f, 'a> Walk<'f, 'a> {
    /// A walk at `scale` pixels to the font unit, the picture's top-left
    /// corner at `corner` from the pen, painting into `picture` - or
    /// measuring, without one.
    fn new(
        face: &'f ttf_parser::Face<'a>,
        scale: f32,
        corner: (f32, f32),
        picture: Option<Pixmap>,
    ) -> Option<Walk<'f, 'a>> {
        let em = scale * face.units_per_em() as f32;
        let surface = match picture {
            Some(base) => Surface::Draw {
                base,
                groups: Vec::new(),
                spoilt: false,
            },
            None => Surface::Measure {
                // Generous for any emoji; it only stops a fill with no
                // clip at all from claiming the whole plane.
                limit: Rect::from_ltrb(-2.0 * em, -3.0 * em, 3.0 * em, 2.0 * em)?,
                ink: None,
            },
        };
        Some(Walk {
            face,
            // The font's y goes up; the picture's goes down.
            transforms: vec![Transform::from_row(
                scale, 0.0, 0.0, -scale, -corner.0, -corner.1,
            )],
            held: None,
            outlined: false,
            nested: false,
            clips: Vec::new(),
            surface,
        })
    }

    fn current(&self) -> Transform {
        self.transforms.last().copied().unwrap_or_default()
    }

    /// Where colour landed, when measuring.
    fn ink(&self) -> Option<Rect> {
        match &self.surface {
            Surface::Measure { ink, .. } => *ink,
            Surface::Draw { .. } => None,
        }
    }

    /// The painted picture, when painting.
    fn finish(self) -> Option<Pixmap> {
        match self.surface {
            Surface::Draw {
                base,
                groups,
                spoilt,
            } => (!spoilt && groups.is_empty()).then_some(base),
            Surface::Measure { .. } => None,
        }
    }

    /// What a fill paints with: a colour, or a gradient laid out in the
    /// font units of the transforms in force.
    fn shader(&self, fill: &ttf_parser::colr::Paint<'a>) -> Option<Shader<'static>> {
        use ttf_parser::colr::Paint as Fill;
        let transform = self.current();
        let coords = self.face.variation_coordinates();
        match fill {
            Fill::Solid(colour) => Some(Shader::SolidColor(colour_of(*colour))),
            Fill::LinearGradient(gradient) => {
                let line = ColourLine::of(gradient.stops(0, coords), gradient.extend)?;
                let p0 = Point::from_xy(gradient.x0, gradient.y0);
                let p1 = Point::from_xy(gradient.x1, gradient.y1);
                // Colour is constant along lines parallel to p0-p2, so the
                // gradient runs from p0 to the foot of p1 on the line
                // through p0 square to them.
                let normal = Point::from_xy(gradient.y2 - gradient.y0, gradient.x0 - gradient.x2);
                let length = normal.x * normal.x + normal.y * normal.y;
                let p3 = if length > 0.0 {
                    let along = ((p1.x - p0.x) * normal.x + (p1.y - p0.y) * normal.y) / length;
                    Point::from_xy(p0.x + normal.x * along, p0.y + normal.y * along)
                } else {
                    p1
                };
                let (from, to) = line.span;
                LinearGradient::new(
                    between(p0, p3, from),
                    between(p0, p3, to),
                    line.stops(),
                    line.mode,
                    transform,
                )
            }
            Fill::RadialGradient(gradient) => {
                let line = ColourLine::of(gradient.stops(0, coords), gradient.extend)?;
                let c0 = Point::from_xy(gradient.x0, gradient.y0);
                let c1 = Point::from_xy(gradient.x1, gradient.y1);
                let (from, to) = line.span;
                let radius = |at: f32| gradient.r0 + (gradient.r1 - gradient.r0) * at;
                RadialGradient::new(
                    between(c0, c1, from),
                    radius(from),
                    between(c0, c1, to),
                    radius(to),
                    line.stops(),
                    line.mode,
                    transform,
                )
            }
            Fill::SweepGradient(gradient) => {
                let mut line = ColourLine::of(gradient.stops(0, coords), gradient.extend)?;
                // Stored in half turns, a half turn short: 0 is 180°.
                let degrees = |value: f32| value * 180.0 + 180.0;
                let (start, end) = (degrees(gradient.start_angle), degrees(gradient.end_angle));
                let (from, to) = line.span;
                let (mut first, mut last) =
                    (start + (end - start) * from, start + (end - start) * to);
                // Angles run counter-clockwise in the font's units, as the
                // rasteriser's do in the space the transform maps from; a
                // sweep that runs backwards is the same sweep turned round.
                if first > last {
                    (first, last) = (last, first);
                    line.reverse();
                }
                if first == last && line.mode != SpreadMode::Pad {
                    return None;
                }
                // The rasteriser maps a sweep's angles onto its stops only
                // when the sweep both starts off zero and spans other than
                // a full turn; a sweep doing just one of those would be
                // drawn round the whole circle. A thousandth of a degree
                // off makes it do both, and shows nowhere.
                const NUDGE: f32 = 1e-3;
                let full_turn = (last - first - 360.0).abs() < NUDGE / 2.0;
                if first == 0.0 && !full_turn {
                    first = NUDGE;
                } else if first != 0.0 && full_turn {
                    last += NUDGE;
                }
                SweepGradient::new(
                    Point::from_xy(gradient.center_x, gradient.center_y),
                    first,
                    last,
                    line.stops(),
                    line.mode,
                    transform,
                )
            }
        }
    }
}

impl<'a> Painter<'a> for Walk<'_, 'a> {
    fn outline_glyph(&mut self, glyph: GlyphId) {
        self.outlined = true;
        let mut units = Units(PathBuilder::new());
        self.held = self
            .face
            .outline_glyph(glyph, &mut units)
            .and_then(|_| units.0.finish())
            .and_then(|path| path.transform(self.current()));
    }

    fn paint(&mut self, fill: ttf_parser::colr::Paint<'a>) {
        let Some(shader) = self.shader(&fill) else {
            return;
        };
        let Walk {
            held,
            outlined,
            nested,
            clips,
            surface,
            ..
        } = self;
        match surface {
            Surface::Measure { limit, ink } => {
                if clips.iter().any(Option::is_none) {
                    return;
                }
                let region = if !clips.is_empty() {
                    clips
                        .iter()
                        .flatten()
                        .map(Path::bounds)
                        .try_fold(*limit, |open, bounds| open.intersect(&bounds))
                } else if !*nested && *outlined {
                    held.as_ref().map(Path::bounds)
                } else {
                    Some(*limit)
                };
                let Some(region) = region.and_then(|region| region.intersect(limit)) else {
                    return;
                };
                if region.width() <= 0.0 || region.height() <= 0.0 {
                    return;
                }
                *ink = Some(match ink {
                    Some(seen) => seen.join(&region).unwrap_or(*seen),
                    None => region,
                });
            }
            Surface::Draw { base, groups, .. } => {
                let paint = Paint {
                    shader,
                    anti_alias: true,
                    ..Paint::default()
                };
                let floor = groups.last().map_or(0, |group| group.clips);
                let target = groups.last_mut().map_or(base, |group| &mut group.pixels);
                let (width, height) = (target.width(), target.height());
                let active = clips.get(floor..).unwrap_or(&[]);
                if let Some((inner, outer)) = active.split_last() {
                    let Some(inner) = inner.as_ref().filter(|path| fillable(path)) else {
                        return;
                    };
                    let Some(mask) = mask_of(outer, width, height) else {
                        return;
                    };
                    target.fill_path(
                        inner,
                        &paint,
                        FillRule::Winding,
                        Transform::identity(),
                        mask.as_ref(),
                    );
                } else if !*nested && *outlined {
                    if let Some(path) = held.as_ref().filter(|path| fillable(path)) {
                        target.fill_path(
                            path,
                            &paint,
                            FillRule::Winding,
                            Transform::identity(),
                            None,
                        );
                    }
                } else if let Some(whole) = Rect::from_xywh(0.0, 0.0, width as f32, height as f32) {
                    target.fill_rect(whole, &paint, Transform::identity(), None);
                }
            }
        }
    }

    fn push_clip(&mut self) {
        self.nested = true;
        self.clips.push(self.held.clone());
    }

    fn push_clip_box(&mut self, clip: ClipBox) {
        self.nested = true;
        let path = Rect::from_ltrb(clip.x_min, clip.y_min, clip.x_max, clip.y_max)
            .map(PathBuilder::from_rect)
            .and_then(|path| path.transform(self.current()));
        self.clips.push(path);
    }

    fn pop_clip(&mut self) {
        self.clips.pop();
    }

    fn push_layer(&mut self, mode: CompositeMode) {
        self.nested = true;
        let clips = self.clips.len();
        if let Surface::Draw {
            base,
            groups,
            spoilt,
        } = &mut self.surface
        {
            match Pixmap::new(base.width(), base.height()) {
                Some(pixels) => groups.push(Group {
                    pixels,
                    mode: blend_of(mode),
                    clips,
                }),
                None => *spoilt = true,
            }
        }
    }

    fn pop_layer(&mut self) {
        let Walk { clips, surface, .. } = self;
        let Surface::Draw {
            base,
            groups,
            spoilt,
        } = surface
        else {
            return;
        };
        if *spoilt {
            return;
        }
        let Some(group) = groups.pop() else {
            return;
        };
        let floor = groups.last().map_or(0, |group| group.clips);
        let target = groups.last_mut().map_or(base, |below| &mut below.pixels);
        let clipping = clips.get(floor..group.clips).unwrap_or(&[]);
        let Some(mask) = mask_of(clipping, target.width(), target.height()) else {
            return;
        };
        target.draw_pixmap(
            0,
            0,
            group.pixels.as_ref(),
            &PixmapPaint {
                blend_mode: group.mode,
                ..PixmapPaint::default()
            },
            Transform::identity(),
            mask.as_ref(),
        );
    }

    fn push_transform(&mut self, transform: ttf_parser::Transform) {
        let ttf_parser::Transform { a, b, c, d, e, f } = transform;
        let next = self
            .current()
            .pre_concat(Transform::from_row(a, b, c, d, e, f));
        self.transforms.push(next);
    }

    fn pop_transform(&mut self) {
        // The first is the font's own scale, never pushed by the glyph.
        if self.transforms.len() > 1 {
            self.transforms.pop();
        }
    }
}

/// A glyph's outline in its font units, as they are.
struct Units(PathBuilder);

impl ttf_parser::OutlineBuilder for Units {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.quad_to(x1, y1, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.cubic_to(x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        self.0.close();
    }
}

/// A gradient's colour stops, in order and spread over 0 to 1, and the
/// stretch of the font's colour line they came from: stops outside 0 to 1
/// move the gradient's ends out to them instead.
struct ColourLine {
    stops: Vec<(f32, Color)>,
    span: (f32, f32),
    mode: SpreadMode,
}

impl ColourLine {
    /// `None` where nothing is to be painted: no stops, or every stop in
    /// one place on a line that repeats or reflects.
    fn of(stops: impl Iterator<Item = ColorStop>, extend: GradientExtend) -> Option<ColourLine> {
        let mode = match extend {
            GradientExtend::Pad => SpreadMode::Pad,
            GradientExtend::Repeat => SpreadMode::Repeat,
            GradientExtend::Reflect => SpreadMode::Reflect,
        };
        let mut stops: Vec<(f32, Color)> = stops
            .map(|stop| (stop.stop_offset, colour_of(stop.color)))
            .collect();
        stops.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (first, mut last) = (stops.first()?.0, stops.last()?.0);
        if last <= first {
            if mode != SpreadMode::Pad {
                return None;
            }
            // Every stop in one place, padded: the first colour up to it
            // and the last past it, which a stop one further along keeps.
            let colour = stops.last()?.1;
            last = first + 1.0;
            stops.push((last, colour));
        }
        let length = last - first;
        for stop in &mut stops {
            stop.0 = (stop.0 - first) / length;
        }
        Some(ColourLine {
            stops,
            span: (first, last),
            mode,
        })
    }

    /// The stops back to front, for the same gradient run the other way.
    fn reverse(&mut self) {
        self.stops.reverse();
        for stop in &mut self.stops {
            stop.0 = 1.0 - stop.0;
        }
    }

    fn stops(&self) -> Vec<GradientStop> {
        self.stops
            .iter()
            .map(|&(at, colour)| GradientStop::new(at, colour))
            .collect()
    }
}

/// The point `at` of the way from `from` to `to`.
fn between(from: Point, to: Point, at: f32) -> Point {
    Point::from_xy(from.x + (to.x - from.x) * at, from.y + (to.y - from.y) * at)
}

fn colour_of(colour: RgbaColor) -> Color {
    Color::from_rgba8(colour.red, colour.green, colour.blue, colour.alpha)
}

/// Whether a path encloses any area at all; one that does not would fill
/// nothing, and the rasteriser declines it.
fn fillable(path: &Path) -> bool {
    let bounds = path.bounds();
    bounds.width() > 1e-3 && bounds.height() > 1e-3
}

/// The clips as one mask over a `width` by `height` surface: `Some(None)`
/// where there are none, `None` where they shut everything out.
fn mask_of(clips: &[Option<Path>], width: u32, height: u32) -> Option<Option<Mask>> {
    let mut mask: Option<Mask> = None;
    for clip in clips {
        let path = clip.as_ref().filter(|path| fillable(path))?;
        match mask.as_mut() {
            Some(mask) => mask.intersect_path(path, FillRule::Winding, true, Transform::identity()),
            None => {
                let mut first = Mask::new(width, height)?;
                first.fill_path(path, FillRule::Winding, true, Transform::identity());
                mask = Some(first);
            }
        }
    }
    Some(mask)
}

fn blend_of(mode: CompositeMode) -> BlendMode {
    match mode {
        CompositeMode::Clear => BlendMode::Clear,
        CompositeMode::Source => BlendMode::Source,
        CompositeMode::Destination => BlendMode::Destination,
        CompositeMode::SourceOver => BlendMode::SourceOver,
        CompositeMode::DestinationOver => BlendMode::DestinationOver,
        CompositeMode::SourceIn => BlendMode::SourceIn,
        CompositeMode::DestinationIn => BlendMode::DestinationIn,
        CompositeMode::SourceOut => BlendMode::SourceOut,
        CompositeMode::DestinationOut => BlendMode::DestinationOut,
        CompositeMode::SourceAtop => BlendMode::SourceAtop,
        CompositeMode::DestinationAtop => BlendMode::DestinationAtop,
        CompositeMode::Xor => BlendMode::Xor,
        CompositeMode::Plus => BlendMode::Plus,
        CompositeMode::Screen => BlendMode::Screen,
        CompositeMode::Overlay => BlendMode::Overlay,
        CompositeMode::Darken => BlendMode::Darken,
        CompositeMode::Lighten => BlendMode::Lighten,
        CompositeMode::ColorDodge => BlendMode::ColorDodge,
        CompositeMode::ColorBurn => BlendMode::ColorBurn,
        CompositeMode::HardLight => BlendMode::HardLight,
        CompositeMode::SoftLight => BlendMode::SoftLight,
        CompositeMode::Difference => BlendMode::Difference,
        CompositeMode::Exclusion => BlendMode::Exclusion,
        CompositeMode::Multiply => BlendMode::Multiply,
        CompositeMode::Hue => BlendMode::Hue,
        CompositeMode::Saturation => BlendMode::Saturation,
        CompositeMode::Color => BlendMode::Color,
        CompositeMode::Luminosity => BlendMode::Luminosity,
    }
}
