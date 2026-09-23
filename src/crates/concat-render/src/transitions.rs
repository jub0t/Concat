// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The transitions a compositor without shaders can still draw.
//!
//! A transition package is a WGSL body, and the CPU reference runs no WGSL.
//! What it runs instead is the shape the package names as its `xfade`: the
//! FFmpeg transition its manifest says it degrades to, drawn here in plain
//! arithmetic over the two finished pictures. Each shape follows the shipped
//! shader that names it rather than FFmpeg's own reading of the name, so a
//! GPU-less export and monitor show the cut the way the GPU does, minus the
//! flourish a shader adds on top: an iris opens from the centre because the
//! iris shader does, a clock wipe sweeps clockwise from twelve, a whip pan
//! carries both pictures leftward. A name this file does not know is a
//! `None`, and the caller keeps the dissolve the incoming clip already
//! carries - the same answer as before there were shapes at all.
//!
//! Pictures are sampled nearest, clamped at their edges, the way the
//! shaders' samplers clamp: a slide that has moved a picture off one side
//! shows its edge pixels smeared, not black, which is what the GPU shows.

use concat_core::frame::Frame;

use crate::compositor::mix_frames;

/// The two pictures combined at `progress` through the shape `xfade`
/// names, or `None` for a shape this file cannot draw or pictures that do
/// not match in size.
pub(crate) fn combine(from: &Frame, to: &Frame, xfade: &str, progress: f32) -> Option<Frame> {
    let (width, height) = (from.width(), from.height());
    if width == 0 || height == 0 || to.width() != width || to.height() != height {
        return None;
    }
    let p = progress.clamp(0.0, 1.0);
    // Most shapes ease the way their shaders do; the crossfades ride the
    // raw progress, which is already the ramp the export planned.
    let s = smoothstep(0.0, 1.0, p);
    let shape = Shape::named(xfade)?;

    let pick = |at: &dyn Fn(f32, f32) -> [f32; 4]| -> Frame {
        let mut out = Frame::black(width, height);
        for y in 0..height {
            let v = (y as f32 + 0.5) / height as f32;
            for x in 0..width {
                let u = (x as f32 + 0.5) / width as f32;
                out.set_pixel(x, y, to_bytes(at(u, v)));
            }
        }
        out
    };
    let from_at = |u: f32, v: f32| sample(from, u, v);
    let to_at = |u: f32, v: f32| sample(to, u, v);

    Some(match shape {
        Shape::Cross => mix_frames(from, to, p),
        Shape::ThroughBlack => {
            // Down to black by the middle, back up out of it.
            let base = if p < 0.5 { from } else { to };
            let dark = if p < 0.5 { 2.0 * p } else { 2.0 - 2.0 * p };
            mix_frames(base, &Frame::black(width, height), dark)
        }
        Shape::ThroughWhite => {
            // The film-burn family: a crossfade with a warm flash peaking
            // at the middle of the cut, the way every shader naming this
            // adds its light over `mix(from, to, progress)`.
            let base = mix_frames(from, to, p);
            let flash = (1.0 - (2.0 * p - 1.0).abs()) * 0.6;
            let warm = [1.0, 0.85, 0.6];
            let mut out = base.clone();
            for pixel in out.pixels_mut().chunks_exact_mut(4) {
                for (channel, tint) in pixel.iter_mut().zip(warm) {
                    let lit = f32::from(*channel) / 255.0 + tint * flash;
                    *channel = (lit.clamp(0.0, 1.0) * 255.0).round() as u8;
                }
            }
            out
        }
        Shape::Clock => pick(&|u, v| {
            // Twelve o'clock is straight up; the hand sweeps clockwise.
            let (cx, cy) = (u - 0.5, v - 0.5);
            let mut angle = cx.atan2(-cy);
            if angle < 0.0 {
                angle += std::f32::consts::TAU;
            }
            if angle < s * std::f32::consts::TAU {
                to_at(u, v)
            } else {
                from_at(u, v)
            }
        }),
        Shape::Iris { closing } => pick(&|u, v| {
            let d =
                ((u - 0.5).powi(2) + (v - 0.5).powi(2)).sqrt() / std::f32::consts::FRAC_1_SQRT_2;
            // The edge overshoots its softness at both ends, so progress
            // 0 is exactly the outgoing picture and 1 exactly the incoming.
            let radius = edge(if closing { 1.0 - s } else { s }, 1.15, 0.05);
            let m = smoothstep(radius - 0.05, radius + 0.05, d);
            // Inside the circle is the incoming picture while it opens, and
            // the outgoing one while it closes.
            let (inside, outside) = if closing {
                (from_at(u, v), to_at(u, v))
            } else {
                (to_at(u, v), from_at(u, v))
            };
            mix(inside, outside, m)
        }),
        Shape::Box { closing } => pick(&|u, v| {
            let d = (u - 0.5).abs().max((v - 0.5).abs()) * 2.0;
            let radius = edge(if closing { 1.0 - s } else { s }, 1.05, 0.03);
            let m = smoothstep(radius - 0.03, radius + 0.03, d);
            let (inside, outside) = if closing {
                (from_at(u, v), to_at(u, v))
            } else {
                (to_at(u, v), from_at(u, v))
            };
            mix(inside, outside, m)
        }),
        Shape::Wipe { coord, soft } => pick(&|u, v| {
            // The incoming picture is uncovered where the coordinate is
            // behind the edge; the edge is as soft as the shape says.
            let at = edge(s, 1.0, soft);
            let m = smoothstep(at - soft, at + soft, coord.of(u, v));
            mix(to_at(u, v), from_at(u, v), m)
        }),
        Shape::Bars { axis, closing } => pick(&|u, v| {
            let d = match axis {
                Axis::X => (u - 0.5).abs() * 2.0,
                Axis::Y => (v - 0.5).abs() * 2.0,
            };
            let at = edge(if closing { 1.0 - s } else { s }, 1.0, 0.02);
            let m = smoothstep(at - 0.02, at + 0.02, d);
            let (inside, outside) = if closing {
                (from_at(u, v), to_at(u, v))
            } else {
                (to_at(u, v), from_at(u, v))
            };
            mix(inside, outside, m)
        }),
        Shape::Slide { dx, dy, moving } => pick(&|u, v| {
            // The incoming picture arrives from the side the direction
            // points away from; under a push the outgoing one leaves by
            // the other side, under a cover it stays put, under a reveal
            // it alone moves.
            // Both pictures travel along (dx, dy): the incoming one is
            // still `1 - s` short of its place, the outgoing one has gone
            // `s` past its own, so each is read from where it was.
            let (tu, tv) = (u + dx * (1.0 - s), v + dy * (1.0 - s));
            let (fu, fv) = (u - dx * s, v - dy * s);
            let arrived = (0.0..=1.0).contains(&tu) && (0.0..=1.0).contains(&tv);
            match moving {
                Moving::Both => {
                    if arrived {
                        to_at(tu, tv)
                    } else {
                        from_at(fu, fv)
                    }
                }
                Moving::Incoming => {
                    if arrived {
                        to_at(tu, tv)
                    } else {
                        from_at(u, v)
                    }
                }
                Moving::Outgoing => {
                    if (0.0..=1.0).contains(&fu) && (0.0..=1.0).contains(&fv) {
                        from_at(fu, fv)
                    } else {
                        to_at(u, v)
                    }
                }
            }
        }),
        Shape::Zoom => pick(&|u, v| {
            // The outgoing picture is pushed into as it goes; the incoming
            // one arrives magnified and settles to its own size.
            let zoom =
                |u: f32, v: f32, scale: f32| ((u - 0.5) / scale + 0.5, (v - 0.5) / scale + 0.5);
            let (fu, fv) = zoom(u, v, 1.0 + s * 1.6);
            let (tu, tv) = zoom(u, v, 1.6 - s * 0.6);
            mix(from_at(fu, fv), to_at(tu, tv), s)
        }),
        Shape::Blocks => {
            // A crossfade through a mosaic that is coarsest at the middle
            // of the cut and gone at either end.
            let k = (p * std::f32::consts::PI).sin();
            let block = (1.0 + k * 0.05 * width.min(height) as f32).floor().max(1.0) as u32;
            pick(&|u, v| {
                let x = ((u * width as f32) as u32 / block * block) as f32 + 0.5;
                let y = ((v * height as f32) as u32 / block * block) as f32 + 0.5;
                let (bu, bv) = (x / width as f32, y / height as f32);
                mix(from_at(bu, bv), to_at(bu, bv), p)
            })
        }
    })
}

/// What a name draws.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Shape {
    Cross,
    ThroughBlack,
    ThroughWhite,
    Clock,
    Iris { closing: bool },
    Box { closing: bool },
    Wipe { coord: Coord, soft: f32 },
    Bars { axis: Axis, closing: bool },
    Slide { dx: f32, dy: f32, moving: Moving },
    Zoom,
    Blocks,
}

/// Which way a wipe's edge travels, as the coordinate that is behind it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Coord {
    /// from the right edge leftward
    Left,
    /// from the left edge rightward
    Right,
    /// from the bottom edge upward
    Up,
    /// from the top edge downward
    Down,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Coord {
    fn of(self, u: f32, v: f32) -> f32 {
        match self {
            Coord::Left => 1.0 - u,
            Coord::Right => u,
            Coord::Up => 1.0 - v,
            Coord::Down => v,
            Coord::TopLeft => (u + v) / 2.0,
            Coord::TopRight => (1.0 - u + v) / 2.0,
            Coord::BottomLeft => (u + 1.0 - v) / 2.0,
            Coord::BottomRight => (2.0 - u - v) / 2.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Axis {
    X,
    Y,
}

/// Which pictures a slide moves.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Moving {
    /// a push: both, in step
    Both,
    /// a cover: the incoming picture over a still outgoing one
    Incoming,
    /// a reveal: the outgoing picture off a still incoming one
    Outgoing,
}

impl Shape {
    /// The shape an FFmpeg `xfade` name means here; see the module.
    fn named(xfade: &str) -> Option<Shape> {
        use Coord::*;
        let hard = 0.004;
        let soft = 0.15;
        let slide = |dx, dy, moving| Shape::Slide { dx, dy, moving };
        Some(match xfade {
            "fade" | "dissolve" | "fadegrays" | "fadefast" | "fadeslow" | "distance" | "hblur" => {
                Shape::Cross
            }
            "fadeblack" => Shape::ThroughBlack,
            "fadewhite" => Shape::ThroughWhite,
            "radial" => Shape::Clock,
            "circleopen" => Shape::Iris { closing: false },
            "circleclose" | "circlecrop" => Shape::Iris { closing: true },
            "rectcrop" => Shape::Box { closing: true },
            "wipeleft" => Shape::Wipe {
                coord: Left,
                soft: hard,
            },
            "wiperight" => Shape::Wipe {
                coord: Right,
                soft: hard,
            },
            "wipeup" => Shape::Wipe {
                coord: Up,
                soft: hard,
            },
            "wipedown" => Shape::Wipe {
                coord: Down,
                soft: hard,
            },
            "wipetl" | "diagtl" => Shape::Wipe {
                coord: TopLeft,
                soft: hard,
            },
            "wipetr" | "diagtr" => Shape::Wipe {
                coord: TopRight,
                soft: hard,
            },
            "wipebl" | "diagbl" => Shape::Wipe {
                coord: BottomLeft,
                soft: hard,
            },
            "wipebr" | "diagbr" => Shape::Wipe {
                coord: BottomRight,
                soft: hard,
            },
            "smoothleft" => Shape::Wipe { coord: Left, soft },
            "smoothright" => Shape::Wipe { coord: Right, soft },
            "smoothup" => Shape::Wipe { coord: Up, soft },
            "smoothdown" => Shape::Wipe { coord: Down, soft },
            "horzopen" => Shape::Bars {
                axis: Axis::Y,
                closing: false,
            },
            "horzclose" => Shape::Bars {
                axis: Axis::Y,
                closing: true,
            },
            "vertopen" => Shape::Bars {
                axis: Axis::X,
                closing: false,
            },
            "vertclose" => Shape::Bars {
                axis: Axis::X,
                closing: true,
            },
            // A slide's direction is where the pictures go: left means the
            // incoming one enters from the right, so its sample point is
            // pushed rightward by what has not yet arrived.
            "slideleft" => slide(-1.0, 0.0, Moving::Both),
            "slideright" => slide(1.0, 0.0, Moving::Both),
            "slideup" => slide(0.0, -1.0, Moving::Both),
            "slidedown" => slide(0.0, 1.0, Moving::Both),
            "coverleft" => slide(-1.0, 0.0, Moving::Incoming),
            "coverright" => slide(1.0, 0.0, Moving::Incoming),
            "coverup" => slide(0.0, -1.0, Moving::Incoming),
            "coverdown" => slide(0.0, 1.0, Moving::Incoming),
            "revealleft" => slide(-1.0, 0.0, Moving::Outgoing),
            "revealright" => slide(1.0, 0.0, Moving::Outgoing),
            "revealup" => slide(0.0, -1.0, Moving::Outgoing),
            "revealdown" => slide(0.0, 1.0, Moving::Outgoing),
            "zoomin" => Shape::Zoom,
            "pixelize" => Shape::Blocks,
            _ => return None,
        })
    }
}

/// The picture's colour at `(u, v)` in `0..=1`, nearest and clamped.
fn sample(picture: &Frame, u: f32, v: f32) -> [f32; 4] {
    let x = ((u * picture.width() as f32).floor().max(0.0) as u32).min(picture.width() - 1);
    let y = ((v * picture.height() as f32).floor().max(0.0) as u32).min(picture.height() - 1);
    let [r, g, b, a] = picture.pixel(x, y).unwrap_or([0, 0, 0, 255]);
    [
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
        f32::from(a) / 255.0,
    ]
}

fn to_bytes(rgba: [f32; 4]) -> [u8; 4] {
    rgba.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// WGSL's `mix`: `a` towards `b` by `t`.
fn mix(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}

/// WGSL's `smoothstep`.
/// Where a soft edge sits at `progress`: it travels from `-soft` to
/// `reach + soft`, so the whole of its blend is off the picture at both
/// ends and progress 0 and 1 are the two pictures themselves.
fn edge(progress: f32, reach: f32, soft: f32) -> f32 {
    progress * (reach + 2.0 * soft) - soft
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    fn solid(rgba: [u8; 4]) -> Frame {
        let mut frame = Frame::black(16, 16);
        frame.fill(rgba);
        frame
    }

    fn at(xfade: &str, progress: f32) -> Frame {
        combine(&solid(RED), &solid(BLUE), xfade, progress).expect("a known shape")
    }

    /// Every shape starts as the outgoing picture and ends as the incoming
    /// one, whatever it does between.
    #[test]
    fn every_shape_runs_from_one_picture_to_the_other() {
        let names = [
            "fade",
            "fadeblack",
            "fadewhite",
            "radial",
            "circleopen",
            "circleclose",
            "rectcrop",
            "wipeleft",
            "wiperight",
            "wipeup",
            "wipedown",
            "wipetl",
            "diagbr",
            "smoothleft",
            "horzopen",
            "horzclose",
            "vertopen",
            "vertclose",
            "slideleft",
            "slideright",
            "slideup",
            "slidedown",
            "coverleft",
            "revealright",
            "zoomin",
            "pixelize",
        ];
        for name in names {
            let start = at(name, 0.0);
            let end = at(name, 1.0);
            for (x, y) in [(0, 0), (8, 8), (15, 15), (3, 12)] {
                assert_eq!(start.pixel(x, y), Some(RED), "{name} at 0, ({x},{y})");
                assert_eq!(end.pixel(x, y), Some(BLUE), "{name} at 1, ({x},{y})");
            }
        }
    }

    /// A name the file does not draw is declined, and so are pictures of
    /// two sizes; the caller keeps its dissolve.
    #[test]
    fn an_unknown_shape_or_a_mismatch_is_declined() {
        assert!(combine(&solid(RED), &solid(BLUE), "hlslice", 0.5).is_none());
        assert!(combine(&solid(RED), &solid(BLUE), "", 0.5).is_none());
        let small = Frame::black(4, 4);
        assert!(combine(&solid(RED), &small, "fade", 0.5).is_none());
        assert!(combine(&Frame::black(0, 0), &Frame::black(0, 0), "fade", 0.5).is_none());
    }

    /// Halfway through a crossfade is halfway between the two, and progress
    /// past either end is held there.
    #[test]
    fn a_crossfade_is_the_mean_at_the_middle() {
        let mid = at("fade", 0.5);
        let [r, _, b, a] = mid.pixel(5, 5).unwrap();
        assert!((126..=129).contains(&r) && (126..=129).contains(&b) && a == 255);
        assert_eq!(at("fade", -3.0).pixel(0, 0), Some(RED));
        assert_eq!(at("fade", 7.0).pixel(0, 0), Some(BLUE));
    }

    /// Through black: dark at the middle, on the way to the other picture.
    #[test]
    fn a_fade_through_black_is_black_at_the_middle() {
        assert_eq!(at("fadeblack", 0.5).pixel(4, 4), Some([0, 0, 0, 255]));
        let [r, _, _, _] = at("fadeblack", 0.25).pixel(4, 4).unwrap();
        assert!((120..=135).contains(&r), "half the red left: {r}");
    }

    /// The clock sweeps clockwise from twelve: at the middle of the cut the
    /// right half has turned over and the left half has not.
    #[test]
    fn a_clock_wipe_sweeps_clockwise_from_twelve() {
        let mid = at("radial", 0.5);
        assert_eq!(mid.pixel(12, 2), Some(BLUE), "top right has turned");
        assert_eq!(mid.pixel(12, 13), Some(BLUE), "bottom right has turned");
        assert_eq!(mid.pixel(3, 13), Some(RED), "bottom left has not");
        assert_eq!(mid.pixel(3, 2), Some(RED), "top left has not");
    }

    /// An iris opens from the centre, so the middle turns over before the
    /// corners; closing, the corners turn over first.
    #[test]
    fn an_iris_opens_from_the_centre() {
        let opening = at("circleopen", 0.5);
        assert_eq!(opening.pixel(8, 8), Some(BLUE));
        assert_eq!(opening.pixel(0, 0), Some(RED));
        let closing = at("circleclose", 0.5);
        assert_eq!(closing.pixel(8, 8), Some(RED));
        assert_eq!(closing.pixel(0, 0), Some(BLUE));
    }

    /// A wipe uncovers the incoming picture behind a moving edge, from the
    /// side the name says.
    #[test]
    fn a_wipe_uncovers_from_the_named_side() {
        let left = at("wipeleft", 0.5);
        assert_eq!(left.pixel(15, 8), Some(BLUE), "right edge went first");
        assert_eq!(left.pixel(0, 8), Some(RED));
        let right = at("wiperight", 0.5);
        assert_eq!(right.pixel(0, 8), Some(BLUE), "left edge went first");
        assert_eq!(right.pixel(15, 8), Some(RED));
        let down = at("wipedown", 0.5);
        assert_eq!(down.pixel(8, 0), Some(BLUE), "top went first");
        assert_eq!(down.pixel(8, 15), Some(RED));
        let corner = at("wipetl", 0.5);
        assert_eq!(
            corner.pixel(0, 0),
            Some(BLUE),
            "the top-left corner went first"
        );
        assert_eq!(corner.pixel(15, 15), Some(RED));
    }

    /// Bars open from the centre line outward, or close onto it.
    #[test]
    fn bars_open_from_the_middle_and_close_onto_it() {
        let open = at("horzopen", 0.5);
        assert_eq!(open.pixel(8, 8), Some(BLUE));
        assert_eq!(open.pixel(8, 0), Some(RED));
        let close = at("vertclose", 0.5);
        assert_eq!(close.pixel(8, 8), Some(RED));
        assert_eq!(close.pixel(0, 8), Some(BLUE));
    }

    /// A slide to the left brings the incoming picture in from the right
    /// while the outgoing one leaves by the left; a cover leaves the
    /// outgoing picture where it is; a reveal moves only the outgoing one.
    #[test]
    fn a_slide_arrives_from_the_far_side() {
        let left = at("slideleft", 0.5);
        assert_eq!(left.pixel(15, 8), Some(BLUE), "arrived on the right");
        assert_eq!(left.pixel(0, 8), Some(RED), "still leaving on the left");
        let up = at("slideup", 0.5);
        assert_eq!(up.pixel(8, 15), Some(BLUE), "arrived from below");
        assert_eq!(up.pixel(8, 0), Some(RED));
        let cover = at("coverright", 0.5);
        assert_eq!(
            cover.pixel(0, 8),
            Some(BLUE),
            "the cover came in from the left"
        );
        assert_eq!(cover.pixel(15, 8), Some(RED));
        let reveal = at("revealdown", 0.5);
        assert_eq!(
            reveal.pixel(8, 0),
            Some(BLUE),
            "the outgoing picture dropped away"
        );
        assert_eq!(reveal.pixel(8, 15), Some(RED));
    }

    /// A mosaic at the middle of a pixelize is still the mean of the two
    /// pictures for pictures that are one colour each, and a zoom likewise:
    /// the shapes move pixels about, and a flat picture has none to move.
    #[test]
    fn a_zoom_and_a_mosaic_over_flat_pictures_are_a_crossfade() {
        for name in ["pixelize", "zoomin"] {
            let [r, _, b, _] = at(name, 0.5).pixel(8, 8).unwrap();
            assert!(
                (100..=155).contains(&r) && (100..=155).contains(&b),
                "{name}: {r} {b}"
            );
        }
    }

    /// The white fade lifts the middle of the cut towards white rather
    /// than blending the two pictures alone.
    #[test]
    fn a_fade_through_white_is_lit_at_the_middle() {
        let [r, g, b, _] = at("fadewhite", 0.5).pixel(8, 8).unwrap();
        let [pr, pg, pb, _] = at("fade", 0.5).pixel(8, 8).unwrap();
        assert!(
            r > pr && g > pg && b >= pb,
            "lit: {r} {g} {b} over {pr} {pg} {pb}"
        );
    }

    /// Sampling clamps at the edges: a picture slid off one side shows its
    /// edge pixels, never a hole.
    #[test]
    fn samples_clamp_at_the_edges() {
        let picture = solid(RED);
        assert_eq!(sample(&picture, -2.0, 0.5), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(sample(&picture, 0.5, 9.0), [1.0, 0.0, 0.0, 1.0]);
    }
}
