// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Brush strokes over a mask.
//!
//! A custom cutout is the model's mask with corrections painted on it.
//! Each stroke is a path of points in source fractions and a diameter as a
//! fraction of the source's width; here it becomes discs along the path,
//! and each disc changes the pixels beneath it the way its tool says:
//!
//! - **Brush** keeps everything under it, **Eraser** removes everything.
//! - **Smart brush** keeps only what the model gave some chance of being
//!   the subject - a sleeve it was unsure of comes back, the wall behind
//!   does not. **Smart eraser** removes only what the model was not sure
//!   of - a bit of chair it half-kept goes, the shoulder beside it stays.
//!
//! The mask is square whatever the picture's shape, so a round brush is an
//! ellipse in mask pixels; `aspect` (width over height of the source) is
//! what makes it round again on screen.

use concat_project::model::{BrushTool, Stroke};

use crate::Mask;

/// Below this much confidence a smart brush leaves a pixel alone.
const SMART_KEEP: u8 = 38;
/// Above this much confidence a smart eraser leaves a pixel alone.
const SMART_DROP: u8 = 217;

/// The model's mask with `strokes` painted over it, in order. `aspect` is
/// the source's width over its height.
pub fn paint(auto: &Mask, strokes: &[Stroke], aspect: f32) -> Mask {
    let mut out = auto.clone();
    for stroke in strokes {
        paint_one(&mut out, auto, stroke, aspect);
    }
    out
}

fn paint_one(out: &mut Mask, auto: &Mask, stroke: &Stroke, aspect: f32) {
    let width = out.width() as f32;
    let height = out.height() as f32;
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        1.0
    };
    // The brush is `size` of the source's width across, and as tall as it
    // is wide on screen: in mask pixels that is an ellipse.
    let rx = (stroke.size as f32 * width / 2.0).max(0.5);
    let ry = (stroke.size as f32 * aspect * height / 2.0).max(0.5);
    let points: Vec<(f32, f32)> = stroke
        .points
        .iter()
        .map(|[x, y]| (*x as f32 * width, *y as f32 * height))
        .collect();
    let Some(&first) = points.first() else {
        return;
    };
    disc(out, auto, stroke.tool, first, rx, ry);
    for pair in points.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let distance = ((to.0 - from.0).powi(2) + (to.1 - from.1).powi(2)).sqrt();
        // A disc every quarter radius, so the path has no scallops.
        let steps = (distance / (rx.min(ry) * 0.25)).ceil().max(1.0) as usize;
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            let centre = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
            disc(out, auto, stroke.tool, centre, rx, ry);
        }
    }
}

fn disc(out: &mut Mask, auto: &Mask, tool: BrushTool, centre: (f32, f32), rx: f32, ry: f32) {
    let width = out.width() as i64;
    let height = out.height() as i64;
    let x0 = ((centre.0 - rx).floor() as i64).max(0);
    let x1 = ((centre.0 + rx).ceil() as i64).min(width - 1);
    let y0 = ((centre.1 - ry).floor() as i64).max(0);
    let y1 = ((centre.1 + ry).ceil() as i64).min(height - 1);
    if x0 > x1 || y0 > y1 {
        return;
    }
    let stride = out.width() as usize;
    let data = out.bytes_mut();
    for y in y0..=y1 {
        let dy = (y as f32 + 0.5 - centre.1) / ry;
        for x in x0..=x1 {
            let dx = (x as f32 + 0.5 - centre.0) / rx;
            if dx * dx + dy * dy > 1.0 {
                continue;
            }
            let index = y as usize * stride + x as usize;
            let was = auto.bytes()[index];
            match tool {
                BrushTool::Brush => data[index] = 255,
                BrushTool::Eraser => data[index] = 0,
                BrushTool::SmartBrush if was >= SMART_KEEP => data[index] = 255,
                BrushTool::SmartEraser if was <= SMART_DROP => data[index] = 0,
                BrushTool::SmartBrush | BrushTool::SmartEraser => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke(tool: BrushTool, size: f64, points: &[[f64; 2]]) -> Stroke {
        Stroke {
            tool,
            size,
            points: points.to_vec(),
        }
    }

    #[test]
    fn a_brush_keeps_and_an_eraser_removes_under_the_path() {
        let auto = Mask::filled(32, 32, 0);
        let kept = paint(
            &auto,
            &[stroke(BrushTool::Brush, 0.25, &[[0.25, 0.5], [0.75, 0.5]])],
            1.0,
        );
        assert_eq!(kept.at(16, 16), 255);
        assert_eq!(kept.at(16, 2), 0);
        assert_eq!(kept.at(2, 2), 0);

        let gone = paint(
            &kept,
            &[stroke(BrushTool::Eraser, 0.25, &[[0.5, 0.5]])],
            1.0,
        );
        assert_eq!(gone.at(16, 16), 0);
        assert_eq!(gone.at(8, 16), 255);
    }

    #[test]
    fn smart_tools_follow_the_models_confidence() {
        let mut auto = Mask::filled(8, 8, 0);
        auto.bytes_mut()[0] = 100; // unsure: rescued by a smart brush
        auto.bytes_mut()[1] = 0; // background: left alone
        auto.bytes_mut()[2] = 250; // sure: kept by a smart eraser
        let whole = [stroke(BrushTool::SmartBrush, 2.0, &[[0.5, 0.5]])];
        let brushed = paint(&auto, &whole, 1.0);
        assert_eq!(brushed.at(0, 0), 255);
        assert_eq!(brushed.at(1, 0), 0);
        let erased = paint(
            &auto,
            &[
                whole[0].clone(),
                stroke(BrushTool::SmartEraser, 2.0, &[[0.5, 0.5]]),
            ],
            1.0,
        );
        // The eraser judges by the model's confidence, not by what an
        // earlier stroke did: the rescued pixel was unsure, so it goes; the
        // sure one the brush also covered stays as the brush left it.
        assert_eq!(erased.at(0, 0), 0);
        assert_eq!(erased.at(2, 0), 255);
    }

    #[test]
    fn the_brush_is_round_on_a_wide_picture() {
        // A 2:1 source squashed into a square mask: a round brush must reach
        // twice as far down the mask as across it.
        let auto = Mask::filled(64, 64, 0);
        let out = paint(&auto, &[stroke(BrushTool::Brush, 0.25, &[[0.5, 0.5]])], 2.0);
        assert_eq!(out.at(32 + 7, 32), 255);
        assert_eq!(out.at(32 + 9, 32), 0);
        assert_eq!(out.at(32, 32 + 15), 255);
        assert_eq!(out.at(32, 32 + 17), 0);
    }
}
