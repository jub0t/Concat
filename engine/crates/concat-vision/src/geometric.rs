// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Source-space geometric masks.
//!
//! A mask is evaluated before the clip is placed, so its position, size and
//! turn travel with the picture through every preview/export compositor. The
//! shape edge is an analytic signed distance where possible; that gives
//! feathering and antialiasing without keeping a second full-frame bitmap.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use concat_core::frame::Frame;
use concat_project::model::{ClipMask, MaskProperty, MaskShape};

use crate::{Mapping, Mask};

/// Applies all enabled masks as one additive alpha matte. An empty Brush or
/// Pen is ignored until the user puts a path in it, so choosing a drawing
/// tool never makes the picture disappear before the first stroke.
pub fn cut(frame: &mut Frame, masks: &[ClipMask], at: f64, text_masks: &BTreeMap<String, Mask>) {
    cut_mapped(frame, masks, &Mapping::IDENTITY, at, text_masks);
}

/// Evaluates authored source-space coordinates through the decoder's crop
/// and flips. The decoded picture's bounds are not the original source's.
pub fn cut_mapped(
    frame: &mut Frame,
    masks: &[ClipMask],
    mapping: &Mapping,
    at: f64,
    text_masks: &BTreeMap<String, Mask>,
) {
    let width = frame.width();
    let height = frame.height();
    if width == 0 || height == 0 {
        return;
    }
    let [left, top, right, bottom] = mapping.crop.map(f64::from);
    let crop_width = (1.0 - left - right).max(0.0);
    let crop_height = (1.0 - top - bottom).max(0.0);
    let source_width = f64::from(width) / crop_width.max(0.1);
    let source_height = f64::from(height) / crop_height.max(0.1);
    let evaluated: Vec<_> = masks
        .iter()
        .filter(|mask| {
            mask.enabled
                && match mask.shape {
                    MaskShape::Brush => !mask.points.is_empty(),
                    MaskShape::Pen => mask.points.len() >= 3,
                    _ => true,
                }
        })
        .map(|mask| {
            Evaluated::new(
                mask,
                at,
                source_width,
                source_height,
                text_masks.get(&mask.id),
            )
        })
        .collect();
    if evaluated.is_empty() {
        return;
    }

    let xs: Vec<f64> = (0..width)
        .map(|x| {
            let fraction = (f64::from(x) + 0.5) / f64::from(width);
            let fraction = if mapping.flip_h {
                1.0 - fraction
            } else {
                fraction
            };
            (left + fraction * crop_width) * source_width
        })
        .collect();
    let ys: Vec<f64> = (0..height)
        .map(|y| {
            let fraction = (f64::from(y) + 0.5) / f64::from(height);
            let fraction = if mapping.flip_v {
                1.0 - fraction
            } else {
                fraction
            };
            (top + fraction * crop_height) * source_height
        })
        .collect();

    let row = width as usize * 4;
    // Borrow once: each `pixels_mut` call updates the frame's upload
    // identity. The old inner-loop call did that once for every pixel.
    let pixels = frame.pixels_mut();
    let workers = if width as usize * height as usize >= 400_000 {
        std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(1)
            .clamp(1, 8)
    } else {
        1
    };
    if workers == 1 {
        cut_band(pixels, row, 0, &xs, &ys, &evaluated);
        return;
    }
    let rows_per = (height as usize).div_ceil(workers);
    std::thread::scope(|scope| {
        for (chunk, band) in pixels.chunks_mut(rows_per * row).enumerate() {
            let first_row = chunk * rows_per;
            let evaluated = &evaluated;
            let xs = &xs;
            let ys = &ys;
            scope.spawn(move || cut_band(band, row, first_row, xs, ys, evaluated));
        }
    });
}

fn cut_band(
    band: &mut [u8],
    row: usize,
    first_row: usize,
    xs: &[f64],
    ys: &[f64],
    evaluated: &[Evaluated<'_>],
) {
    for (row_index, line) in band.chunks_exact_mut(row).enumerate() {
        let y = ys[first_row + row_index];
        for (x, pixel) in line.chunks_exact_mut(4).enumerate() {
            let mut matte = 0.0_f32;
            for mask in evaluated {
                matte = matte.max(mask.coverage(xs[x], y));
            }
            pixel[3] = (f32::from(pixel[3]) * matte.clamp(0.0, 1.0)).round() as u8;
        }
    }
}

struct Evaluated<'a> {
    source: &'a ClipMask,
    text: Option<&'a Mask>,
    cx: f64,
    cy: f64,
    width: f64,
    height: f64,
    min_dimension: f64,
    sin: f64,
    cos: f64,
    feather_pixels: f64,
    roundness: f64,
    points: Vec<(f64, f64)>,
}

impl<'a> Evaluated<'a> {
    fn new(
        mask: &'a ClipMask,
        at: f64,
        frame_width: f64,
        frame_height: f64,
        text: Option<&'a Mask>,
    ) -> Self {
        let value = |property: MaskProperty| mask.value_at(property, at);
        let width = value(MaskProperty::Width).clamp(0.01, 4.0) * frame_width;
        let height = value(MaskProperty::Height).clamp(0.01, 4.0) * frame_height;
        let angle = value(MaskProperty::Rotation).to_radians();
        let (sin, cos) = angle.sin_cos();
        Self {
            source: mask,
            text,
            cx: frame_width * (0.5 + value(MaskProperty::PositionX) * 0.5),
            cy: frame_height * (0.5 + value(MaskProperty::PositionY) * 0.5),
            width,
            height,
            min_dimension: width.min(height).max(1.0),
            sin,
            cos,
            feather_pixels: value(MaskProperty::Feather).clamp(0.0, 0.5)
                * frame_width.min(frame_height),
            roundness: value(MaskProperty::Roundness).clamp(0.0, 1.0),
            points: mask
                .points
                .iter()
                .map(|[x, y]| (*x - 0.5, *y - 0.5))
                .collect(),
        }
    }

    /// The point in a unit shape whose boundary normally lies at ±0.5.
    fn local(&self, x: f64, y: f64) -> (f64, f64) {
        let dx = x - self.cx;
        let dy = y - self.cy;
        let x = dx * self.cos + dy * self.sin;
        let y = -dx * self.sin + dy * self.cos;
        (x / self.width, y / self.height)
    }

    fn coverage(&self, x: f64, y: f64) -> f32 {
        let (x, y) = self.local(x, y);
        let mut coverage = match self.source.shape {
            MaskShape::Split => self.from_distance(y * self.height),
            MaskShape::Filmstrip => {
                let mut bands = 0.0_f64;
                for centre in [-0.34_f64, 0.0, 0.34] {
                    let radius = self.roundness * 0.12;
                    let qx = x.abs() - (0.5 - radius);
                    let qy = (y - centre).abs() - (0.12 - radius);
                    let distance = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius;
                    bands = bands.max(self.from_distance(distance * self.min_dimension));
                }
                bands
            }
            MaskShape::Rectangle => {
                let radius = self.roundness * 0.5;
                let qx = x.abs() - (0.5 - radius);
                let qy = y.abs() - (0.5 - radius);
                let outside = qx.max(0.0).hypot(qy.max(0.0));
                let inside = qx.max(qy).min(0.0);
                self.from_distance((outside + inside - radius) * self.min_dimension)
            }
            MaskShape::Circle => {
                self.from_distance(((x * 2.0).hypot(y * 2.0) - 1.0) * self.min_dimension * 0.5)
            }
            MaskShape::Star => self.from_distance(star_field().sample(x, y) * self.min_dimension),
            MaskShape::Heart => self.from_distance(heart_field().sample(x, y) * self.min_dimension),
            MaskShape::Text => self.text_coverage(x, y, self.text),
            MaskShape::Brush => self.brush_coverage(x, y),
            MaskShape::Pen => self.polygon_coverage(x, y, &self.points),
        };
        if self.source.inverted {
            coverage = 1.0 - coverage;
        }
        coverage.clamp(0.0, 1.0) as f32
    }

    fn from_distance(&self, signed: f64) -> f64 {
        let softness = self.feather_pixels.max(0.75);
        (0.5 - signed / (2.0 * softness)).clamp(0.0, 1.0)
    }

    fn far_from_unit_box(&self, x: f64, y: f64, extra: f64) -> bool {
        let margin = extra + self.feather_pixels.max(0.75) / self.min_dimension;
        x.abs() > 0.5 + margin || y.abs() > 0.5 + margin
    }

    fn polygon_coverage(&self, x: f64, y: f64, points: &[(f64, f64)]) -> f64 {
        if self.far_from_unit_box(x, y, 0.0) {
            0.0
        } else {
            self.from_distance(polygon_distance(x, y, points) * self.min_dimension)
        }
    }

    fn brush_coverage(&self, x: f64, y: f64) -> f64 {
        let radius = self.source.brush_size.clamp(0.002, 1.0) * 0.5;
        if self.far_from_unit_box(x, y, radius) {
            return 0.0;
        }
        let mut distance_squared = f64::INFINITY;
        for pair in self.points.windows(2) {
            if pair[0].0 < -0.5 || pair[1].0 < -0.5 {
                continue;
            }
            distance_squared =
                distance_squared.min(segment_distance_squared((x, y), pair[0], pair[1]));
        }
        for (px, py) in self.points.iter().filter(|point| point.0 >= -0.5) {
            distance_squared = distance_squared.min((x - px).powi(2) + (y - py).powi(2));
        }
        self.from_distance((distance_squared.sqrt() - radius) * self.min_dimension)
    }

    fn text_coverage(&self, x: f64, y: f64, text: Option<&Mask>) -> f64 {
        let Some(text) = text else { return 0.0 };
        let sample = |x: f64, y: f64| text.sample((x + 0.5) as f32, (y + 0.5) as f32) as f64;
        if self.feather_pixels <= 0.75 {
            return sample(x, y);
        }
        let radius_x = self.feather_pixels / self.width.max(1.0);
        let radius_y = self.feather_pixels / self.height.max(1.0);
        let offsets = [
            (0.0, 0.0),
            (-radius_x, 0.0),
            (radius_x, 0.0),
            (0.0, -radius_y),
            (0.0, radius_y),
            (-radius_x * 0.7, -radius_y * 0.7),
            (radius_x * 0.7, -radius_y * 0.7),
            (-radius_x * 0.7, radius_y * 0.7),
            (radius_x * 0.7, radius_y * 0.7),
        ];
        offsets
            .iter()
            .map(|(dx, dy)| sample(x + dx, y + dy))
            .sum::<f64>()
            / offsets.len() as f64
    }
}

fn segment_distance_squared(point: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let ab = (b.0 - a.0, b.1 - a.1);
    let length = ab.0 * ab.0 + ab.1 * ab.1;
    if length <= f64::EPSILON {
        return (point.0 - a.0).powi(2) + (point.1 - a.1).powi(2);
    }
    let t = (((point.0 - a.0) * ab.0 + (point.1 - a.1) * ab.1) / length).clamp(0.0, 1.0);
    (point.0 - (a.0 + ab.0 * t)).powi(2) + (point.1 - (a.1 + ab.1 * t)).powi(2)
}

/// Negative inside, positive outside a closed polygon.
fn polygon_distance(x: f64, y: f64, points: &[(f64, f64)]) -> f64 {
    if points.len() < 3 {
        return f64::INFINITY;
    }
    let mut inside = false;
    let mut distance_squared = f64::INFINITY;
    for index in 0..points.len() {
        let a = points[index];
        let b = points[(index + 1) % points.len()];
        distance_squared = distance_squared.min(segment_distance_squared((x, y), a, b));
        if ((a.1 > y) != (b.1 > y)) && x < (b.0 - a.0) * (y - a.1) / (b.1 - a.1) + a.0 {
            inside = !inside;
        }
    }
    let distance = distance_squared.sqrt();
    if inside { -distance } else { distance }
}

/// A canonical signed-distance map for complex built-in outlines. Computing
/// 64 segment distances for every pixel of every animated frame made a
/// 960×540 Heart mask take hundreds of milliseconds; moving/rotating the
/// mask should only resample its unchanged local shape.
const FIELD_SIDE: usize = 512;

struct DistanceField {
    signed: Box<[f32]>,
}

impl DistanceField {
    fn new(points: &[(f64, f64)]) -> Self {
        let signed = (0..FIELD_SIDE)
            .flat_map(|y| {
                (0..FIELD_SIDE).map(move |x| {
                    let x = x as f64 * 2.0 / (FIELD_SIDE - 1) as f64 - 1.0;
                    let y = y as f64 * 2.0 / (FIELD_SIDE - 1) as f64 - 1.0;
                    polygon_distance(x, y, points) as f32
                })
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self { signed }
    }

    fn sample(&self, x: f64, y: f64) -> f64 {
        // The outlines live within ±0.5. Outside this wider domain even
        // heavy feathering is background; avoid a clamped edge sample.
        let outside = (x.abs() - 1.0).max(y.abs() - 1.0).max(0.0);
        if outside > 0.0 {
            return 0.5 + outside;
        }
        let scale = (FIELD_SIDE - 1) as f64 / 2.0;
        let px = (x + 1.0) * scale;
        let py = (y + 1.0) * scale;
        let x0 = px.floor() as usize;
        let y0 = py.floor() as usize;
        let x1 = (x0 + 1).min(FIELD_SIDE - 1);
        let y1 = (y0 + 1).min(FIELD_SIDE - 1);
        let fx = (px - x0 as f64) as f32;
        let fy = (py - y0 as f64) as f32;
        let at = |x: usize, y: usize| self.signed[y * FIELD_SIDE + x];
        let top = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
        let bottom = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
        f64::from(top * (1.0 - fy) + bottom * fy)
    }
}

fn star_field() -> &'static DistanceField {
    static FIELD: OnceLock<DistanceField> = OnceLock::new();
    FIELD.get_or_init(|| DistanceField::new(star_points()))
}

fn heart_field() -> &'static DistanceField {
    static FIELD: OnceLock<DistanceField> = OnceLock::new();
    FIELD.get_or_init(|| DistanceField::new(heart_points()))
}

fn star_points() -> &'static [(f64, f64)] {
    static POINTS: OnceLock<Vec<(f64, f64)>> = OnceLock::new();
    POINTS
        .get_or_init(|| {
            (0..10)
                .map(|index| {
                    let angle =
                        -std::f64::consts::FRAC_PI_2 + index as f64 * std::f64::consts::PI / 5.0;
                    let radius = if index % 2 == 0 { 0.49 } else { 0.22 };
                    (radius * angle.cos(), radius * angle.sin())
                })
                .collect()
        })
        .as_slice()
}

fn heart_points() -> &'static [(f64, f64)] {
    static POINTS: OnceLock<Vec<(f64, f64)>> = OnceLock::new();
    POINTS
        .get_or_init(|| {
            (0..64)
                .map(|index| {
                    let t = index as f64 / 64.0 * std::f64::consts::TAU;
                    let x = 16.0 * t.sin().powi(3) / 34.0;
                    let y = -(13.0 * t.cos()
                        - 5.0 * (2.0 * t).cos()
                        - 2.0 * (3.0 * t).cos()
                        - (4.0 * t).cos())
                        / 34.0;
                    (x, y)
                })
                .collect()
        })
        .as_slice()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_outline_distances_track_the_analytic_shapes() {
        for (field, outline) in [
            (heart_field(), heart_points()),
            (star_field(), star_points()),
        ] {
            for (x, y) in [
                (0.0, 0.0),
                (0.4, 0.2),
                (-0.3, 0.1),
                (0.15, -0.45),
                (0.6, 0.5),
            ] {
                let analytic = polygon_distance(x, y, outline);
                assert!(
                    (field.sample(x, y) - analytic).abs() < 0.01,
                    "{x} {y}: cached {} vs analytic {analytic}",
                    field.sample(x, y)
                );
            }
        }
    }

    #[test]
    fn geometric_mask_follows_source_crop_and_flip() {
        let mut mask = ClipMask::new("shape".to_owned(), MaskShape::Circle);
        mask.position_x = 0.8;
        mask.width = 0.15;
        mask.height = 0.3;
        let mapping = Mapping {
            crop: [0.5, 0.0, 0.0, 0.0],
            flip_h: true,
            flip_v: false,
        };
        let mut frame = Frame::black(64, 64);
        cut_mapped(&mut frame, &[mask], &mapping, 0.0, &BTreeMap::new());
        assert!(frame.pixel(13, 32).unwrap()[3] > 200);
        assert_eq!(frame.pixel(51, 32).unwrap()[3], 0);
    }

    fn masked(shape: MaskShape, inverted: bool) -> Frame {
        let mut frame = Frame::from_rgba(32, 32, vec![255; 32 * 32 * 4]).unwrap();
        let mut mask = ClipMask::new("mask1".to_owned(), shape);
        mask.inverted = inverted;
        cut(&mut frame, &[mask], 0.0, &BTreeMap::new());
        frame
    }

    #[test]
    fn a_circle_keeps_its_centre_and_drops_its_corners() {
        let frame = masked(MaskShape::Circle, false);
        assert!(frame.pixel(16, 16).unwrap()[3] > 240);
        assert_eq!(frame.pixel(0, 0).unwrap()[3], 0);
    }

    #[test]
    fn inversion_flips_the_geometric_matte() {
        let frame = masked(MaskShape::Rectangle, true);
        assert_eq!(frame.pixel(16, 16).unwrap()[3], 0);
        assert!(frame.pixel(0, 0).unwrap()[3] > 240);
    }

    #[test]
    fn a_mask_uses_its_independent_position_keys() {
        let mut frame = Frame::from_rgba(32, 32, vec![255; 32 * 32 * 4]).unwrap();
        let mut mask = ClipMask::new("mask1".to_owned(), MaskShape::Circle);
        mask.width = 0.2;
        mask.height = 0.2;
        mask.set_key(
            MaskProperty::PositionX,
            0.0,
            0.0,
            concat_project::model::KeyEase::LINEAR,
        );
        mask.set_key(
            MaskProperty::PositionX,
            1.0,
            0.5,
            concat_project::model::KeyEase::LINEAR,
        );
        cut(&mut frame, &[mask], 1.0, &BTreeMap::new());
        assert_eq!(frame.pixel(16, 16).unwrap()[3], 0);
        assert!(frame.pixel(24, 16).unwrap()[3] > 240);
    }

    #[test]
    fn empty_drawing_masks_leave_the_picture_alone() {
        let mut frame = Frame::from_rgba(8, 8, vec![255; 8 * 8 * 4]).unwrap();
        let mask = ClipMask::new("mask1".to_owned(), MaskShape::Brush);
        cut(&mut frame, &[mask], 0.0, &BTreeMap::new());
        assert_eq!(frame.pixel(0, 0).unwrap()[3], 255);
    }

    #[test]
    fn an_incomplete_pen_does_not_blank_the_clip() {
        let mut mask = ClipMask::new("pen".to_owned(), MaskShape::Pen);
        let points = [[0.2, 0.2], [0.8, 0.2], [0.5, 0.8]];
        for count in 1..=2 {
            mask.points = points[..count].to_vec();
            let mut frame = Frame::black(32, 32);
            cut(&mut frame, &[mask.clone()], 0.0, &BTreeMap::new());
            assert_eq!(frame.pixel(0, 0).unwrap()[3], 255, "{count} point(s)");
        }
        mask.points = points.to_vec();
        let mut frame = Frame::black(32, 32);
        cut(&mut frame, &[mask], 0.0, &BTreeMap::new());
        assert!(frame.pixel(16, 16).unwrap()[3] > 200);
        assert_eq!(frame.pixel(0, 0).unwrap()[3], 0);
    }
}
