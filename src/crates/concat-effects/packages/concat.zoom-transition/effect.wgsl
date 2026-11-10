// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Zoom: the camera rushes forward (or back) the whole way through. The
// outgoing shot is magnified until it is gone, and the incoming one is
// met already moving, small in a mirrored field, and arrives at its own
// size. A radial blur follows the speed, which peaks at the swap and hides
// it.

struct Params { direction: f32 }

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

// The magnification each half reaches: two and a half times.
const SPAN: f32 = 0.9162907;
const TAPS: i32 = 12;

// One picture magnified by exp(`zoom`) about the centre.
fn zoomed(uv: vec2<f32>, zoom: f32, side: i32) -> vec4<f32> {
    let at = fold((uv - vec2<f32>(0.5)) * exp(-zoom) + vec2<f32>(0.5));
    var colour = from_at(at);
    if (side == 1) {
        colour = to_at(at);
    }
    return vec4<f32>(colour.rgb * colour.a, colour.a);
}

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = clamp(progress, 0.0, 1.0);
    let sense = select(1.0, -1.0, params.direction > 0.5);
    let e = ease(p);
    // The camera's place on one log scale, from the outgoing shot at its
    // size (0) through the swap (SPAN) to the incoming one at its own.
    var zoom = 2.0 * SPAN * e;
    var side = 0;
    if (e >= 0.5) {
        side = 1;
        zoom = zoom - 2.0 * SPAN;
    }
    // The blur's reach, in log zoom: the camera's speed over a frame of a
    // one-second cut, and more.
    let speed = 2.0 * SPAN * 6.0 * p * (1.0 - p);
    let reach = speed / 12.0;
    var gathered = vec4<f32>(0.0);
    for (var k = 0; k < TAPS; k++) {
        let off = (f32(k) / f32(TAPS - 1) - 0.5) * reach;
        gathered += zoomed(uv, sense * (zoom + off), side);
    }
    let mean = gathered / f32(TAPS);
    return vec4<f32>(mean.rgb / max(mean.a, 1e-6), mean.a);
}
