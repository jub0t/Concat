// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The CPU's effects: one kernel per built-in package, keyed by its id.
//!
//! A package is a WGSL shader, and the GPU runs it. The CPU reference
//! cannot run WGSL at any useful speed, so it carries a kernel of its own
//! for each package it knows, written to the same arithmetic as the
//! shader and read from the same resolved values, and checked against the
//! GPU by the parity suite. A package it does not know - a community
//! package, or a built-in nobody has ported - is drawn untreated and said
//! so once per process, which is honest where a filter string that
//! silently differs was not.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use concat_core::frame::{BYTES_PER_PIXEL, Frame};
use concat_core::shader::ShaderPass;

use crate::reference::sample_bilinear;

/// Every package the CPU renders.
pub const KERNELS: &[&str] = &[
    "concat.black-white",
    "concat.invert",
    "concat.sepia",
    "concat.hue-shift",
    "concat.vignette",
    "concat.box-blur",
    "concat.gaussian-blur",
];

/// Whether the CPU has a kernel for the package.
pub fn has_kernel(package: &str) -> bool {
    KERNELS.contains(&package)
}

/// Runs the kernel for `pass.package` over `picture` at full strength, the
/// intensity mix being the caller's; `None` when there is no kernel for
/// the package. `seconds` is the timeline instant, for a kernel that
/// moves; none of these do.
pub fn run(pass: &ShaderPass, picture: &Frame, seconds: f32) -> Option<Frame> {
    let _ = seconds;
    let knob = |key: &str| pass.values.get(key).copied().unwrap_or(0.0) as f32;
    Some(match pass.package.as_str() {
        "concat.black-white" => per_pixel(picture, |[r, g, b, a]| {
            let y = luma(r, g, b);
            [y, y, y, a]
        }),
        "concat.invert" => per_pixel(picture, |[r, g, b, a]| [1.0 - r, 1.0 - g, 1.0 - b, a]),
        "concat.sepia" => per_pixel(picture, |[r, g, b, a]| {
            [
                (r * 0.393 + g * 0.769 + b * 0.189).clamp(0.0, 1.0),
                (r * 0.349 + g * 0.686 + b * 0.168).clamp(0.0, 1.0),
                (r * 0.272 + g * 0.534 + b * 0.131).clamp(0.0, 1.0),
                a,
            ]
        }),
        "concat.hue-shift" => {
            let (sn, cs) = knob("angle").to_radians().sin_cos();
            per_pixel(picture, |[r, g, b, a]| {
                // A turn of the hue is a plain rotation in YIQ, where the
                // two chroma axes are at right angles.
                let y = 0.299 * r + 0.587 * g + 0.114 * b;
                let i = 0.596 * r - 0.274 * g - 0.322 * b;
                let q = 0.211 * r - 0.523 * g + 0.312 * b;
                let (i, q) = (i * cs - q * sn, i * sn + q * cs);
                [
                    (y + 0.956 * i + 0.621 * q).clamp(0.0, 1.0),
                    (y - 0.272 * i - 0.647 * q).clamp(0.0, 1.0),
                    (y - 1.106 * i + 1.703 * q).clamp(0.0, 1.0),
                    a,
                ]
            })
        }
        "concat.vignette" => {
            let s = knob("strength") / 100.0;
            per_point(picture, |[r, g, b, a], u, v| {
                let d = ((u - 0.5) * 2.0).hypot((v - 0.5) * 2.0);
                let fall = smoothstep(1.4 - s * 0.9, 1.4 + 0.2 - s * 0.3, d);
                let keep = 1.0 - fall * (0.4 + 0.6 * s);
                [r * keep, g * keep, b * keep, a]
            })
        }
        "concat.box-blur" => {
            let step = (knob("radius") / 4.0).max(0.5);
            gathered(picture, step, |_, _| 1.0)
        }
        "concat.gaussian-blur" => {
            let sigma = knob("radius").max(0.5);
            let step = (sigma / 2.5).max(1.0);
            gathered(picture, step, move |dx, dy| {
                (-(dx * dx + dy * dy) / (2.0 * sigma * sigma)).exp()
            })
        }
        _ => return None,
    })
}

/// Says once, in the log, that `package` is drawn untreated on the CPU.
pub fn fallback_once(package: &str) {
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| Mutex::new(HashSet::new()));
    let mut seen = seen.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if seen.insert(package.to_owned()) {
        log::warn!("{package}: no CPU kernel for this package; the picture is drawn untreated");
    }
}

/// Luminance, Rec. 709.
fn luma(r: f32, g: f32, b: f32) -> f32 {
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// WGSL's `smoothstep`.
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Every pixel through `f`, straight colour and alpha in `0..=1`.
fn per_pixel(picture: &Frame, f: impl Fn([f32; 4]) -> [f32; 4]) -> Frame {
    per_point(picture, |rgba, _, _| f(rgba))
}

/// Every pixel through `f` with where it is, `u` and `v` in `0..1` across
/// the picture at the pixel's centre.
fn per_point(picture: &Frame, f: impl Fn([f32; 4], f32, f32) -> [f32; 4]) -> Frame {
    let (width, height) = (picture.width(), picture.height());
    let mut out = picture.clone();
    let stride = width as usize * BYTES_PER_PIXEL;
    let pixels = out.pixels_mut();
    for y in 0..height as usize {
        let v = (y as f32 + 0.5) / height as f32;
        for x in 0..width as usize {
            let u = (x as f32 + 0.5) / width as f32;
            let at = y * stride + x * BYTES_PER_PIXEL;
            let rgba = f(
                [
                    f32::from(pixels[at]) / 255.0,
                    f32::from(pixels[at + 1]) / 255.0,
                    f32::from(pixels[at + 2]) / 255.0,
                    f32::from(pixels[at + 3]) / 255.0,
                ],
                u,
                v,
            );
            for (channel, value) in rgba.iter().enumerate() {
                pixels[at + channel] = (value * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// The blurs' 9×9 gather: every pixel becomes the weighted mean of the
/// samples `step` pixels apart around it, `weight` of the offset in
/// pixels, sampled bilinearly with the edge clamped the way the GPU's
/// sampler does.
fn gathered(picture: &Frame, step: f32, weight: impl Fn(f32, f32) -> f32) -> Frame {
    let (width, height) = (picture.width(), picture.height());
    let mut out = picture.clone();
    let stride = width as usize * BYTES_PER_PIXEL;
    let taps: Vec<(f32, f32, f32)> = (-4..=4)
        .flat_map(|y| (-4..=4).map(move |x| (x as f32 * step, y as f32 * step)))
        .map(|(dx, dy)| (dx, dy, weight(dx, dy)))
        .collect();
    let total: f32 = taps.iter().map(|(_, _, w)| *w).sum();
    let pixels = out.pixels_mut();
    for y in 0..height as usize {
        for x in 0..width as usize {
            let mut sum = [0.0f32; 4];
            for (dx, dy, w) in &taps {
                let sample = sample_bilinear(picture, x as f32 + dx, y as f32 + dy);
                for (channel, value) in sample.iter().enumerate() {
                    sum[channel] += value * w;
                }
            }
            let at = y * stride + x * BYTES_PER_PIXEL;
            for (channel, value) in sum.iter().enumerate() {
                pixels[at + channel] = (value / total).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use super::*;

    fn pass(package: &str, values: &[(&str, f64)]) -> ShaderPass {
        ShaderPass {
            package: package.to_owned(),
            key: package.to_owned(),
            source: Arc::from(""),
            params: vec![0; ShaderPass::MIN_PARAMS],
            values: values
                .iter()
                .map(|(key, value)| ((*key).to_owned(), *value))
                .collect::<BTreeMap<_, _>>(),
            intensity: 1.0,
            lut: None,
            reveal_map: None,
        }
    }

    fn solid(rgba: [u8; 4]) -> Frame {
        let mut frame = Frame::transparent(4, 4);
        frame.fill(rgba);
        frame
    }

    #[test]
    fn every_listed_package_runs_and_an_unknown_one_does_not() {
        let picture = solid([200, 100, 50, 255]);
        for package in KERNELS {
            assert!(has_kernel(package));
            let knobs = [("radius", 2.0), ("strength", 50.0), ("angle", 90.0)];
            let out = run(&pass(package, &knobs), &picture, 0.0)
                .unwrap_or_else(|| panic!("{package} has a kernel"));
            assert_eq!((out.width(), out.height()), (4, 4));
        }
        assert!(!has_kernel("someone.else"));
        assert!(run(&pass("someone.else", &[]), &picture, 0.0).is_none());
        fallback_once("someone.else");
        fallback_once("someone.else");
    }

    #[test]
    fn the_colour_kernels_do_their_arithmetic() {
        let red = solid([255, 0, 0, 255]);
        let bw = run(&pass("concat.black-white", &[]), &red, 0.0).expect("kernel");
        let y = (0.2126f32 * 255.0).round() as u8;
        assert_eq!(bw.pixel(0, 0), Some([y, y, y, 255]));
        let inverted = run(&pass("concat.invert", &[]), &red, 0.0).expect("kernel");
        assert_eq!(inverted.pixel(0, 0), Some([0, 255, 255, 255]));
        let sepia = run(&pass("concat.sepia", &[]), &red, 0.0).expect("kernel");
        assert_eq!(sepia.pixel(0, 0), Some([100, 89, 69, 255]));
        // A full turn of the hue is the colour again, near enough.
        let turned =
            run(&pass("concat.hue-shift", &[("angle", 360.0)]), &red, 0.0).expect("kernel");
        let [r, g, b, _] = turned.pixel(0, 0).expect("in");
        assert!(r >= 250 && g <= 5 && b <= 5, "{r} {g} {b}");
        // Alpha rides through every colour kernel untouched.
        let half = solid([255, 0, 0, 128]);
        assert_eq!(
            run(&pass("concat.invert", &[]), &half, 0.0)
                .expect("kernel")
                .pixel(0, 0),
            Some([0, 255, 255, 128])
        );
    }

    #[test]
    fn a_blur_of_a_flat_picture_is_the_picture_and_the_edges_stay_put() {
        let flat = solid([90, 120, 30, 255]);
        for package in ["concat.box-blur", "concat.gaussian-blur"] {
            let out = run(&pass(package, &[("radius", 8.0)]), &flat, 0.0).expect("kernel");
            assert_eq!(out.pixel(0, 0), Some([90, 120, 30, 255]), "{package}");
            assert_eq!(out.pixel(3, 3), Some([90, 120, 30, 255]), "{package}");
        }
        // A negative radius is the floor, not a panic.
        let out = run(&pass("concat.box-blur", &[("radius", -9.0)]), &flat, 0.0).expect("kernel");
        assert_eq!(out.pixel(1, 1), Some([90, 120, 30, 255]));
        let mut edge = Frame::transparent(4, 1);
        edge.set_pixel(0, 0, [255, 255, 255, 255]);
        let out = run(&pass("concat.box-blur", &[("radius", 4.0)]), &edge, 0.0).expect("kernel");
        let [near, ..] = out.pixel(0, 0).expect("in");
        let [far, ..] = out.pixel(3, 0).expect("in");
        assert!(
            near > far,
            "the white spreads but stays brightest where it was: {near} vs {far}"
        );
    }

    #[test]
    fn a_vignette_darkens_the_corners_and_not_the_centre() {
        let mut flat = Frame::transparent(64, 64);
        flat.fill([200, 200, 200, 255]);
        let out =
            run(&pass("concat.vignette", &[("strength", 100.0)]), &flat, 0.0).expect("kernel");
        let [centre, ..] = out.pixel(32, 32).expect("in");
        let [corner, ..] = out.pixel(0, 0).expect("in");
        assert!(centre >= 195, "{centre}");
        assert!(corner < centre, "{corner} < {centre}");
        let out = run(&pass("concat.vignette", &[("strength", 0.0)]), &flat, 0.0).expect("kernel");
        assert_eq!(
            out.pixel(0, 0),
            Some([200, 200, 200, 255]),
            "no strength, no fall"
        );
    }
}
