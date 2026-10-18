// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! A clip's mask on the monitor: its outline and its grips, and what a
//! drag on one of them does to it.
//!
//! The mask lives on the picture as it is seen (see `model::ClipMask`): its
//! centre a fraction of the picture's width and height from the middle,
//! its box in the picture's shorter side, its turn on top of the clip's.
//! This is that arithmetic run forwards into the frame, for the outline and
//! the grips, and backwards, for a drag. All in frame pixels, where a turn
//! is a turn; the overlay gets fractions of the frame.
//!
//! A picture pulled along one axis (`stretch_x` != `stretch_y`) draws its
//! mask pulled along with it, which a rotated outline here does not follow:
//! the grips are placed for the unpulled picture. Rare, and said here.

use concat_core::mask::{self, MaskUniform};
use concat_project::model::{ClipMask, MaskShape};

use super::Footprint;

/// Which grip a press landed on: what the overlay numbers them, from 20.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MaskGrip {
    /// The centre: the whole mask moves.
    Move,
    /// The right edge's middle: the width.
    Width,
    /// The bottom edge's middle: the height, a band's thickness.
    Height,
    /// The bottom-right corner: both, keeping their proportion.
    Corner,
    /// The handle under the mask: the turn.
    Rotate,
    /// The handle above the mask, at the feather's outer edge: the softness.
    Feather,
}

impl MaskGrip {
    /// The first number the overlay gives a mask grip; the clip's own box
    /// has the ones below.
    pub const FIRST: i32 = 20;

    /// The grip the overlay's number names, if it is a mask's.
    pub fn of(number: i32) -> Option<MaskGrip> {
        Some(match number - Self::FIRST {
            0 => MaskGrip::Move,
            1 => MaskGrip::Width,
            2 => MaskGrip::Height,
            3 => MaskGrip::Corner,
            4 => MaskGrip::Rotate,
            5 => MaskGrip::Feather,
            _ => return None,
        })
    }
}

/// A mask placed in the frame: where its picture is and how it is turned,
/// and the mask's numbers at one instant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaskFrame {
    /// The mask's numbers, keyed ones at their ride's value.
    pub mask: ClipMask,
    /// The picture's centre, in frame pixels.
    picture: (f64, f64),
    /// The picture's turn, radians.
    picture_turn: f64,
    /// The picture's size as drawn, in frame pixels.
    size: (f64, f64),
    /// The picture's shorter side as drawn, in frame pixels: the mask's
    /// unit of length.
    short: f64,
    /// The frame's size, for turning pixels into fractions.
    frame: (f64, f64),
}

impl MaskFrame {
    /// The mask on a picture whose box in the frame is `footprint`, in a
    /// frame `frame` pixels; `stretch` is the picture's pull along each
    /// axis, which its shorter side is measured without.
    pub fn new(
        mask: ClipMask,
        footprint: &Footprint,
        frame: (u32, u32),
        stretch: (f64, f64),
    ) -> MaskFrame {
        let (width, height) = (f64::from(frame.0), f64::from(frame.1));
        let size = (footprint.w * width, footprint.h * height);
        let short = (size.0 / stretch.0.max(1e-6))
            .min(size.1 / stretch.1.max(1e-6))
            .max(1.0);
        MaskFrame {
            mask,
            picture: (footprint.cx * width, footprint.cy * height),
            picture_turn: footprint.rotation.to_radians(),
            size,
            short,
            frame: (width, height),
        }
    }

    /// The mask's centre, in frame pixels.
    pub fn centre(&self) -> (f64, f64) {
        let along = (self.mask.x * self.size.0, self.mask.y * self.size.1);
        let (sin, cos) = self.picture_turn.sin_cos();
        (
            self.picture.0 + along.0 * cos - along.1 * sin,
            self.picture.1 + along.0 * sin + along.1 * cos,
        )
    }

    /// The mask's whole turn in the frame: the picture's and its own.
    fn turn(&self) -> f64 {
        self.picture_turn + self.mask.rotation.to_radians()
    }

    /// A point in the mask's own frame - pixels across and down from its
    /// centre, before its turn - as frame pixels.
    fn point(&self, local: (f64, f64)) -> (f64, f64) {
        let (cx, cy) = self.centre();
        let (sin, cos) = self.turn().sin_cos();
        (
            cx + local.0 * cos - local.1 * sin,
            cy + local.0 * sin + local.1 * cos,
        )
    }

    /// A frame pixel in the mask's own frame: the inverse of `point`.
    fn local(&self, at: (f64, f64)) -> (f64, f64) {
        let (cx, cy) = self.centre();
        let (dx, dy) = (at.0 - cx, at.1 - cy);
        let (sin, cos) = self.turn().sin_cos();
        (dx * cos + dy * sin, -dx * sin + dy * cos)
    }

    /// Half the box, in frame pixels: none across for a line or a band,
    /// none down for a line.
    fn half(&self) -> (f64, f64) {
        let shape = self.mask.shape.engine();
        let across = if shape.has_width() {
            self.mask.width * self.short / 2.0
        } else {
            0.0
        };
        let down = if shape.has_height() {
            self.mask.height * self.short / 2.0
        } else {
            0.0
        };
        (across, down)
    }

    /// How far the turn and feather handles stand off the box.
    fn stem(&self) -> f64 {
        (self.short * 0.08).max(12.0)
    }

    /// The grips, in frame pixels, in [`MaskGrip`] order from `Move`.
    pub fn grips(&self) -> [(f64, f64); 6] {
        let (hw, hh) = self.half();
        let band = self.mask.feather * self.short * 0.5;
        [
            self.point((0.0, 0.0)),
            self.point((hw, 0.0)),
            self.point((0.0, hh)),
            self.point((hw, hh)),
            self.point((0.0, hh + self.stem() * 1.6)),
            self.point((0.0, -(hh + band / 2.0 + self.stem()))),
        ]
    }

    /// A point of the frame as the overlay's path takes it: a thousandth
    /// of the frame's width across and of its height down.
    fn path_point(&self, at: (f64, f64)) -> String {
        format!(
            "{:.2} {:.2}",
            at.0 / self.frame.0 * 1000.0,
            at.1 / self.frame.1 * 1000.0
        )
    }

    /// The mask's outline as path commands on the overlay's thousand-unit
    /// box: a line or two across the picture for a line and a band, the
    /// edge itself for the rest - traced from the same distance the
    /// compositor draws, so a heart's outline is the heart that is kept.
    pub fn outline(&self) -> String {
        let (_, hh) = self.half();
        // Long enough to cross any picture the mask sits on.
        let reach = self.size.0.hypot(self.size.1);
        match self.mask.shape {
            MaskShape::Linear => format!(
                "M {} L {}",
                self.path_point(self.point((-reach, 0.0))),
                self.path_point(self.point((reach, 0.0)))
            ),
            MaskShape::Mirror => format!(
                "M {} L {} M {} L {}",
                self.path_point(self.point((-reach, -hh))),
                self.path_point(self.point((reach, -hh))),
                self.path_point(self.point((-reach, hh))),
                self.path_point(self.point((reach, hh)))
            ),
            _ => {
                let mut out = String::new();
                for (index, local) in self.edge(96).into_iter().enumerate() {
                    let verb = if index == 0 { "M" } else { "L" };
                    out.push_str(&format!("{verb} {} ", self.path_point(self.point(local))));
                }
                out.push('Z');
                out
            }
        }
    }

    /// `count` points round the shape's edge in its own frame, in pixels:
    /// along each ray from the centre, where the distance changes sign.
    fn edge(&self, count: usize) -> Vec<(f64, f64)> {
        let uniform: MaskUniform = self.mask.engine().uniform(1.0, 1.0, 0.0);
        let [code, _, _, radius] = uniform.a;
        let half = [uniform.b[2], uniform.b[3]];
        let reach = f64::from(half[0].max(half[1])) * 2.0;
        (0..count)
            .map(|index| {
                let angle = index as f64 / count as f64 * std::f64::consts::TAU;
                let (sin, cos) = angle.sin_cos();
                let inside = |r: f64| {
                    mask::distance(code, [(cos * r) as f32, (sin * r) as f32], half, radius) < 0.0
                };
                // Walk out to the first point outside, then halve the step.
                let steps = 64;
                let mut outer = reach;
                for step in 1..=steps {
                    let r = reach * step as f64 / steps as f64;
                    if !inside(r) {
                        outer = r;
                        break;
                    }
                }
                let mut inner = (outer - reach / steps as f64).max(0.0);
                for _ in 0..20 {
                    let middle = (inner + outer) / 2.0;
                    if inside(middle) {
                        inner = middle;
                    } else {
                        outer = middle;
                    }
                }
                let r = (inner + outer) / 2.0 * self.short;
                (cos * r, sin * r)
            })
            .collect()
    }

    /// The mask after `grip` is dragged from `from` to `to`, frame pixels;
    /// `self` is the mask as the press found it.
    pub fn dragged(&self, grip: MaskGrip, from: (f64, f64), to: (f64, f64)) -> ClipMask {
        let mut next = self.mask;
        match grip {
            MaskGrip::Move => {
                let (cx, cy) = self.centre();
                let moved = (
                    cx + to.0 - from.0 - self.picture.0,
                    cy + to.1 - from.1 - self.picture.1,
                );
                let (sin, cos) = self.picture_turn.sin_cos();
                let along = (
                    moved.0 * cos + moved.1 * sin,
                    -moved.0 * sin + moved.1 * cos,
                );
                next.x = along.0 / self.size.0.max(1.0);
                next.y = along.1 / self.size.1.max(1.0);
            }
            MaskGrip::Width => {
                next.width = 2.0 * self.local(to).0.abs() / self.short;
            }
            MaskGrip::Height => {
                next.height = 2.0 * self.local(to).1.abs() / self.short;
            }
            MaskGrip::Corner => {
                let before = self.local(from);
                let after = self.local(to);
                let ratio = after.0.hypot(after.1) / before.0.hypot(before.1).max(1.0);
                next.width = self.mask.width * ratio;
                next.height = self.mask.height * ratio;
            }
            MaskGrip::Rotate => {
                let (cx, cy) = self.centre();
                let swept = (to.1 - cy).atan2(to.0 - cx) - (from.1 - cy).atan2(from.0 - cx);
                let turned = self.mask.rotation + swept.to_degrees();
                next.rotation = (turned + 180.0).rem_euclid(360.0) - 180.0;
            }
            MaskGrip::Feather => {
                // Up the mask's own axis is softer.
                let pulled = self.local(from).1 - self.local(to).1;
                next.feather = self.mask.feather + pulled / (self.short * 0.5);
            }
        }
        next.tidy()
    }

    /// A point of the frame in pixels, as the overlay's fractions.
    pub fn fraction(&self, at: (f64, f64)) -> (f32, f32) {
        ((at.0 / self.frame.0) as f32, (at.1 / self.frame.1) as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 200 by 100 picture filling a 200 by 100 frame, unturned.
    fn whole() -> Footprint {
        Footprint {
            cx: 0.5,
            cy: 0.5,
            w: 1.0,
            h: 1.0,
            rotation: 0.0,
        }
    }

    fn placed(mask: ClipMask) -> MaskFrame {
        MaskFrame::new(mask, &whole(), (200, 100), (1.0, 1.0))
    }

    #[test]
    fn the_grips_sit_on_the_box_in_the_shorter_side() {
        let frame = placed(ClipMask {
            x: 0.25,
            width: 0.5,
            height: 0.4,
            ..ClipMask::of(MaskShape::Rectangle)
        });
        let grips = frame.grips();
        // A quarter of 200 right of the middle; the short side is 100.
        assert_eq!(grips[0], (150.0, 50.0));
        assert_eq!(grips[1], (175.0, 50.0));
        assert_eq!(grips[2], (150.0, 70.0));
        assert_eq!(grips[3], (175.0, 70.0));
    }

    #[test]
    fn a_drag_on_each_grip_does_what_it_says() {
        let frame = placed(ClipMask {
            width: 0.5,
            height: 0.5,
            ..ClipMask::of(MaskShape::Circle)
        });
        let moved = frame.dragged(MaskGrip::Move, (100.0, 50.0), (120.0, 40.0));
        assert!((moved.x - 0.1).abs() < 1e-9 && (moved.y + 0.1).abs() < 1e-9);
        let wider = frame.dragged(MaskGrip::Width, (125.0, 50.0), (140.0, 50.0));
        assert!((wider.width - 0.8).abs() < 1e-9);
        assert_eq!(wider.height, 0.5);
        let bigger = frame.dragged(MaskGrip::Corner, (125.0, 75.0), (150.0, 100.0));
        assert!((bigger.width - 1.0).abs() < 1e-9 && (bigger.height - 1.0).abs() < 1e-9);
        let turned = frame.dragged(MaskGrip::Rotate, (100.0, 90.0), (60.0, 50.0));
        assert!((turned.rotation - 90.0).abs() < 1e-6, "{}", turned.rotation);
        let softer = frame.dragged(MaskGrip::Feather, (100.0, 10.0), (100.0, 0.0));
        assert!((softer.feather - 0.2).abs() < 1e-9);
    }

    #[test]
    fn a_turned_picture_turns_its_mask_and_its_drags() {
        let footprint = Footprint {
            rotation: 90.0,
            ..whole()
        };
        let frame = MaskFrame::new(
            ClipMask {
                x: 0.25,
                ..ClipMask::of(MaskShape::Circle)
            },
            &footprint,
            (200, 100),
            (1.0, 1.0),
        );
        // A quarter of the picture's width to its right is down the frame
        // once the picture is turned a quarter clockwise.
        let (cx, cy) = frame.centre();
        assert!((cx - 100.0).abs() < 1e-9 && (cy - 100.0).abs() < 1e-9);
        // Dragging it back up to the middle puts it back in the middle.
        let back = frame.dragged(MaskGrip::Move, (100.0, 100.0), (100.0, 50.0));
        assert!(back.x.abs() < 1e-9 && back.y.abs() < 1e-9);
    }

    #[test]
    fn the_outline_traces_the_edge_the_compositor_keeps() {
        let frame = placed(ClipMask {
            width: 0.6,
            height: 0.6,
            ..ClipMask::of(MaskShape::Circle)
        });
        for (x, y) in frame.edge(16) {
            assert!((x.hypot(y) - 30.0).abs() < 0.01, "a circle of 30 pixels");
        }
        let star = placed(ClipMask::of(MaskShape::Star)).outline();
        assert!(star.starts_with("M ") && star.ends_with('Z'));
        let line = placed(ClipMask::of(MaskShape::Linear)).outline();
        assert_eq!(line.matches('L').count(), 1);
        assert_eq!(
            placed(ClipMask::of(MaskShape::Mirror))
                .outline()
                .matches('M')
                .count(),
            2
        );
    }
}
