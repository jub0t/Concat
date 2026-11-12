// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Swirl: a whirlpool in the middle of the frame. Points near the centre
// turn furthest and the edges hardly at all; the twist winds up to the
// middle of the cut and lets go after it, and the shots trade places at
// its tightest.

struct Params { twist: f32 }

// Nought at both ends of the cut, one at its middle.
fn bell(p: f32) -> f32 {
    return sin(3.14159265 * clamp(p, 0.0, 1.0));
}
// Two colours mixed by `w`, premultiplied, in light: halfway from white
// to black is half the light, as a dissolve between two exposures is.
fn blend(a: vec4<f32>, b: vec4<f32>, w: f32) -> vec4<f32> {
    let alpha = mix(a.a, b.a, w);
    let rgb = mix(a.rgb * a.a, b.rgb * b.a, w);
    return vec4<f32>(rgb / max(alpha, 1e-6), alpha);
}
// A point in pixels about the centre of the frame, and back.
fn centred(uv: vec2<f32>) -> vec2<f32> {
    return (uv - vec2<f32>(0.5)) * frame.size;
}

fn uncentred(d: vec2<f32>) -> vec2<f32> {
    return d / frame.size + vec2<f32>(0.5);
}

// The whirl's reach: the half-diagonal, so it touches the corners.
fn reach() -> f32 {
    return 0.5 * length(frame.size);
}

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = clamp(progress, 0.0, 1.0);
    let d = centred(uv);
    let r = length(d) / reach();
    let fall = (1.0 - min(r, 1.0)) * (1.0 - min(r, 1.0));
    // Up to two and a half turns at the centre, at full twist.
    let angle = clamp(params.twist, 0.0, 100.0) / 100.0 * 15.7079633 * bell(p) * fall;
    let c = cos(angle);
    let s = sin(angle);
    let at = uncentred(vec2<f32>(c * d.x - s * d.y, s * d.x + c * d.y));
    let w = smoothstep(0.35, 0.65, p);
    return blend(from_at(at), to_at(at), w);
}
