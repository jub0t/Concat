// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Titles, shapes and a caption lane's look.

use concat_doc::model::{
    CaptionLook, Color, Gradient, LineHead, ShapeKind, ShapeShadow, ShapeStyle, StrokeAlign,
    StrokeDash, TextAlign, TextCase, TextRun, Title,
};
use serde_json::Value;

use crate::build::Build;
use crate::read::{Fields, Kept, color};

/// The least a size above zero is taken to be.
const TINY: f64 = 1e-6;

/// A colour field: `#rrggbb[aa]`, or `or` when there is none or it does
/// not read, said.
fn colour_or(
    build: &mut Build,
    place: &str,
    fields: &mut Fields<'_>,
    key: &str,
    or: Color,
) -> Color {
    match fields.str(key) {
        None => or,
        Some(text) => match color(text) {
            Some(colour) => colour,
            None => {
                build.note(
                    place,
                    format!("its {key} \"{text}\" is not a colour; it is the default now"),
                );
                or
            }
        },
    }
}

/// A colour that may be empty for none.
fn colour_opt(
    build: &mut Build,
    place: &str,
    fields: &mut Fields<'_>,
    key: &str,
    or: Option<Color>,
) -> Option<Color> {
    match fields.str(key) {
        None => or,
        Some(text) if text.trim().is_empty() => None,
        Some(text) => match color(text) {
            Some(colour) => Some(colour),
            None => {
                build.note(
                    place,
                    format!("its {key} \"{text}\" is not a colour; it is the default now"),
                );
                or
            }
        },
    }
}

/// A title's style, the fields left over kept under `path`.
pub(crate) fn title(
    build: &mut Build,
    place: &str,
    value: &Value,
    kept: &mut Kept,
    path: &str,
) -> Title {
    let base = Title::default();
    let mut f = Fields::of(value);
    let content = f.string_or("content", &base.content);
    let font_family = f.string_or("fontFamily", &base.font_family);
    let font_size = f.f64_or("fontSize", base.font_size);
    let font_size = build.within(place, "font size", font_size, TINY, f64::MAX);
    // The earlier reader held a weight and a line height to these, so the
    // title showed them so.
    let font_weight = f.f64_or("fontWeight", base.font_weight).clamp(100.0, 900.0);
    let line_height = f.f64_or("lineHeight", base.line_height).max(0.5);
    let italic = f.bool_or("italic", base.italic);
    let underline = f.bool_or("underline", base.underline);
    let strikethrough = f.bool_or("strikethrough", base.strikethrough);
    let color = colour_or(build, place, &mut f, "color", base.color);
    let opacity = f.f64_or("opacity", base.opacity);
    let opacity = build.within(place, "title opacity", opacity, 0.0, 1.0);
    let align = match f.str("align") {
        None | Some("center") => TextAlign::Center,
        Some("left") => TextAlign::Left,
        Some("right") => TextAlign::Right,
        // The earlier reader centred what it did not know, and so does
        // this; the word is kept.
        Some(other) => {
            kept.keep(path, "align", Value::String(other.to_owned()));
            TextAlign::Center
        }
    };
    let case = match f.str("textCase") {
        None | Some("original") => TextCase::Original,
        Some("upper") => TextCase::Upper,
        Some("lower") => TextCase::Lower,
        Some("sentence") => TextCase::Sentence,
        Some(other) => TextCase::Unknown(build.raw(&Value::String(other.to_owned()))),
    };
    let tracking = f.f64_or("tracking", base.tracking);
    let max_width = f.f64_or("maxWidth", base.max_width).max(0.0);
    let max_height = f.f64_or("maxHeight", base.max_height).max(0.0);
    let stroke_color = colour_or(build, place, &mut f, "strokeColor", base.stroke_color);
    let stroke_width = f.f64_or("strokeWidth", base.stroke_width);
    let stroke_width = build.within(place, "outline width", stroke_width, 0.0, f64::MAX);
    let shadow = f.bool_or("shadow", base.shadow);
    let plate = colour_opt(build, place, &mut f, "background", None);
    let plate_padding_x = f
        .f64_or("backgroundPaddingX", base.plate_padding_x)
        .max(0.0);
    let plate_padding_y = f
        .f64_or("backgroundPaddingY", base.plate_padding_y)
        .max(0.0);
    let plate_radius = f.f64_or("backgroundRadius", base.plate_radius).max(0.0);
    let chars = u32::try_from(content.chars().count()).unwrap_or(u32::MAX);
    let mut runs: Vec<TextRun> = Vec::new();
    let mut dropped = 0;
    for (at, run) in f.list("runs").iter().enumerate() {
        match text_run(run, kept, &format!("{path}/runs/{at}")) {
            Some(run) => runs.push(run),
            None => {
                dropped += 1;
                kept.keep(&format!("{path}/runs"), &at.to_string(), run.clone());
            }
        }
    }
    // Kept in order, apart and inside the words.
    runs.sort_by_key(|run| run.start);
    let mut after = 0;
    runs.retain(|run| {
        let fits = run.start < run.end && run.start >= after && run.end <= chars;
        if fits {
            after = run.end;
        } else {
            dropped += 1;
        }
        fits
    });
    if dropped > 0 {
        build.note(
            place,
            format!(
                "{dropped} of its styled runs of words did not fit its words or overlapped another, and are left in its own look"
            ),
        );
    }
    kept.rest(path, &f);
    Title {
        content,
        font_family,
        font_size,
        font_weight,
        italic,
        underline,
        strikethrough,
        color,
        opacity,
        align,
        case,
        line_height,
        tracking,
        max_width,
        max_height,
        stroke_color,
        stroke_width,
        shadow,
        plate,
        plate_padding_x,
        plate_padding_y,
        plate_radius,
        runs,
    }
}

fn text_run(value: &Value, kept: &mut Kept, path: &str) -> Option<TextRun> {
    let mut f = Fields::of(value);
    if !f.has("start") || !f.has("end") {
        return None;
    }
    let start = f.u32("start")?;
    let end = f.u32("end")?;
    let colour = |f: &mut Fields<'_>, key: &str| f.str(key).and_then(color);
    let run = TextRun {
        start,
        end,
        color: colour(&mut f, "color"),
        font_scale: f.f64("fontScale").map(|s| s.clamp(0.1, 10.0)),
        font_weight: f.f64("fontWeight").map(|w| w.clamp(100.0, 900.0)),
        italic: f.bool("italic"),
        underline: f.bool("underline"),
        strikethrough: f.bool("strikethrough"),
        stroke_color: colour(&mut f, "strokeColor"),
    };
    kept.rest(path, &f);
    Some(run)
}

/// A caption lane's look: a title's style with no words of its own.
pub(crate) fn caption_look(
    build: &mut Build,
    place: &str,
    value: &Value,
    kept: &mut Kept,
) -> CaptionLook {
    let mut f = Fields::of(value);
    let offset_y = f.f64_or("offsetY", 0.0);
    let mut title = match f.object("style") {
        Some(style) => self::title(build, place, style, kept, "captions/style"),
        None => CaptionLook::default().title,
    };
    // Each caption's words, and the words styled apart, are its own.
    title.content.clear();
    title.runs.clear();
    kept.rest("captions", &f);
    CaptionLook { title, offset_y }
}

fn line_head(build: &mut Build, value: Option<&str>) -> Option<LineHead> {
    Some(match value? {
        "none" => LineHead::None,
        "arrow" => LineHead::Arrow,
        "triangle" => LineHead::Triangle,
        "circle" => LineHead::Circle,
        "bar" => LineHead::Bar,
        other => LineHead::Unknown(build.raw(&Value::String(other.to_owned()))),
    })
}

/// A shape's figure and paint, the fields left over kept under `path`.
pub(crate) fn shape(
    build: &mut Build,
    place: &str,
    value: &Value,
    kept: &mut Kept,
    path: &str,
) -> ShapeStyle {
    let base = ShapeStyle::default();
    let mut f = Fields::of(value);
    let kind = match f.str("kind") {
        None | Some("square") => ShapeKind::Square,
        Some("circle") => ShapeKind::Circle,
        Some("triangle") => ShapeKind::Triangle,
        Some("parallelogram") => ShapeKind::Parallelogram,
        Some("trapezoid") => ShapeKind::Trapezoid,
        Some("line") => ShapeKind::Line,
        Some("arrow") => ShapeKind::Arrow,
        Some("polygon") => ShapeKind::Polygon,
        Some("star") => ShapeKind::Star,
        Some("heart") => ShapeKind::Heart,
        Some(other) => ShapeKind::Unknown(build.raw(&Value::String(other.to_owned()))),
    };
    // `size` is what files before a height called the width.
    let width = match f.f64("width") {
        Some(width) => width,
        None => f.f64_or("size", base.width),
    };
    let width = build.within(place, "shape width", width, 0.0, f64::MAX);
    let height = f.f64_or("height", base.height);
    let height = build.within(place, "shape height", height, 0.0, f64::MAX);
    let corner_radius = f.f64_or("cornerRadius", base.corner_radius);
    let corner_radius = build.within(place, "corner roundness", corner_radius, 0.0, 1.0);
    let sides = f.u32("sides").unwrap_or(base.sides).clamp(3, 12);
    let points = f.u32("points").unwrap_or(base.points).clamp(3, 24);
    let inner_radius = f.f64_or("innerRadius", base.inner_radius);
    let inner_radius = build.within(place, "star depth", inner_radius, 0.0, 1.0);
    let fill = colour_opt(build, place, &mut f, "fill", base.fill);
    let gradient = f.object("gradient").map(|g| {
        let mut g = Fields::of(g);
        let gradient = Gradient {
            color: g.str("color").and_then(color).unwrap_or(Color::BLACK),
            angle: g.f64_or("angle", 90.0),
            radial: g.bool_or("radial", false),
        };
        kept.rest(&format!("{path}/gradient"), &g);
        gradient
    });
    let stroke = colour_opt(build, place, &mut f, "stroke", None);
    let stroke_width = f.f64_or("strokeWidth", base.stroke_width);
    let stroke_width = build.within(place, "outline width", stroke_width, 0.0, f64::MAX);
    let stroke_align = match f.str("strokeAlign") {
        None | Some("center") => StrokeAlign::Center,
        Some("inside") => StrokeAlign::Inside,
        Some("outside") => StrokeAlign::Outside,
        Some(other) => StrokeAlign::Unknown(build.raw(&Value::String(other.to_owned()))),
    };
    let dash = match f.str("dash") {
        None | Some("solid") => StrokeDash::Solid,
        Some("dashed") => StrokeDash::Dashed,
        Some("dotted") => StrokeDash::Dotted,
        Some(other) => StrokeDash::Unknown(build.raw(&Value::String(other.to_owned()))),
    };
    let trim_start = f.f64_or("trimStart", base.trim_start);
    let trim_start = build.within(place, "outline start", trim_start, 0.0, 1.0);
    let trim_end = f.f64_or("trimEnd", base.trim_end);
    let trim_end = build.within(place, "outline end", trim_end, 0.0, 1.0);
    let start_head = f.str("startHead").map(Some);
    let start_head = line_head(build, start_head.flatten());
    let end_head = f.str("endHead").map(Some);
    let end_head = line_head(build, end_head.flatten());
    let shadow = f.object("shadow").map(|s| {
        let mut s = Fields::of(s);
        let shadow = ShapeShadow {
            color: s.str("color").and_then(color).unwrap_or(Color {
                r: 0,
                g: 0,
                b: 0,
                a: 0x99,
            }),
            angle: s.f64_or("angle", 90.0),
            distance: s.f64_or("distance", 0.012).max(0.0),
            blur: s.f64_or("blur", 0.02).max(0.0),
        };
        kept.rest(&format!("{path}/shadow"), &s);
        shadow
    });
    kept.rest(path, &f);
    ShapeStyle {
        kind,
        width,
        height,
        corner_radius,
        sides,
        points,
        inner_radius,
        fill,
        gradient,
        stroke,
        stroke_width,
        stroke_align,
        dash,
        trim_start,
        trim_end,
        start_head,
        end_head,
        shadow,
    }
}
