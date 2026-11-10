// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Zoom Blur: a lunge and a recoil. The camera drives into the outgoing
// shot and the light streaks out from the centre; the incoming shot starts
// as close and pulls back to its own size, the streaks dying as it slows.

struct Params { strength: f32 }

// Smoothstep's curve: leaves and lands with no jolt.
fn ease(p: f32) -> f32 {
    let q = clamp(p, 0.0, 1.0);
    return q * q * (3.0 - 2.0 * q);
}
// Nought at both ends of the cut, one at its middle.
fn bell(p: f32) -> f32 {
    return sin(3.14159265 * clamp(p, 0.0, 1.0));
}
// A point folded back into the picture, as a mirror at each edge would,
// so a picture moved or shrunk past its frame has no empty corners.
fn fold(uv: vec2<f32>) -> vec2<f32> {
    let m = abs(uv - 2.0 * floor(uv * 0.5 + vec2<f32>(0.5)));
    return clamp(m, vec2<f32>(0.0), vec2<f32>(1.0));
}

const TAPS: i32 = 16;

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = clamp(progress, 0.0, 1.0);
    let strength = clamp(params.strength, 0.0, 100.0) / 100.0;
    // Magnification climbs to three times at full strength, at the swap.
    let peak = log(1.0 + 2.0 * strength);
    var zoom = peak * ease(p * 2.0);
    var side = 0;
    if (p >= 0.5) {
        side = 1;
        zoom = peak * (1.0 - ease(p * 2.0 - 1.0));
    }
    // Streak length, as a share of each point's distance from the centre.
    let streak = 0.35 * strength * bell(p);
    var gathered = vec4<f32>(0.0);
    for (var k = 0; k < TAPS; k++) {
        let s = f32(k) / f32(TAPS - 1);
        let at = fold((uv - vec2<f32>(0.5)) * exp(-zoom) * (1.0 - streak * s) + vec2<f32>(0.5));
        var c = from_at(at);
        if (side == 1) {
            c = to_at(at);
        }
        gathered += vec4<f32>(c.rgb * c.a, c.a);
    }
    let mean = gathered / f32(TAPS);
    return vec4<f32>(mean.rgb / max(mean.a, 1e-6), mean.a);
}
