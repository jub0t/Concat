// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Spin: the picture turns about the centre of the frame, the outgoing
// shot for the first half and the incoming one for the second, so the
// incoming shot finishes the turn the outgoing one began. It is blurred
// round the turn as fast as it spins, and the corners a turned frame opens
// are filled by mirroring the picture, never left black.

struct Params { turns: f32 }

// Smoothstep's curve: leaves and lands with no jolt.
fn ease(p: f32) -> f32 {
    let q = clamp(p, 0.0, 1.0);
    return q * q * (3.0 - 2.0 * q);
}
// A point folded back into the picture, as a mirror at each edge would,
// so a picture moved or shrunk past its frame has no empty corners.
fn fold(uv: vec2<f32>) -> vec2<f32> {
    let m = abs(uv - 2.0 * floor(uv * 0.5 + vec2<f32>(0.5)));
    return clamp(m, vec2<f32>(0.0), vec2<f32>(1.0));
}
// A point in pixels about the centre of the frame, and back.
fn centred(uv: vec2<f32>) -> vec2<f32> {
    return (uv - vec2<f32>(0.5)) * frame.size;
}

fn uncentred(d: vec2<f32>) -> vec2<f32> {
    return d / frame.size + vec2<f32>(0.5);
}

const TAPS: i32 = 11;

// One picture turned by `angle` radians about the centre.
fn turned(uv: vec2<f32>, angle: f32, side: i32) -> vec4<f32> {
    let d = centred(uv);
    let along = vec2<f32>(cos(angle), sin(angle));
    let at = fold(uncentred(vec2<f32>(along.x * d.x + along.y * d.y, along.x * d.y - along.y * d.x)));
    var colour = from_at(at);
    if (side == 1) {
        colour = to_at(at);
    }
    return vec4<f32>(colour.rgb * colour.a, colour.a);
}

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = clamp(progress, 0.0, 1.0);
    let whole = clamp(params.turns, 0.5, 2.0) * 6.28318531;
    // The incoming shot ends upright: its angle runs to a whole number
    // of the turn's length past where it started.
    let angle = whole * ease(p);
    var side = 0;
    var shown = angle;
    if (p >= 0.5) {
        side = 1;
        shown = angle - whole;
    }
    // The blur's arc: how far the frame turns in about a frame of a
    // one-second cut, at this instant.
    let rate = whole * 6.0 * p * (1.0 - p);
    let arc = rate / 30.0;
    var gathered = vec4<f32>(0.0);
    for (var k = 0; k < TAPS; k++) {
        let off = (f32(k) / f32(TAPS - 1) - 0.5) * arc;
        gathered += turned(uv, shown + off, side);
    }
    let mean = gathered / f32(TAPS);
    return vec4<f32>(mean.rgb / max(mean.a, 1e-6), mean.a);
}
