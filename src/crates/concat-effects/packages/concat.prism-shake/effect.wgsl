// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Prism Shake: a jitter where the three channels each move on noise of
// their own about a shared tremble, so edges break into colour. No
// shutter: the split is the point, and a smear would muddy it.

struct Params { amount: f32, speed: f32, split: f32 }

// A point folded back into the picture, as a mirror at each edge would.
fn fold(uv: vec2<f32>) -> vec2<f32> {
    let m = abs(uv - 2.0 * floor(uv * 0.5 + vec2<f32>(0.5)));
    return clamp(m, vec2<f32>(0.0), vec2<f32>(1.0));
}

// Where the picture under `uv` comes from when the camera has moved by
// `offset` (in frame heights), turned by `angle` and zoomed by `zoom`.
fn through(uv: vec2<f32>, offset: vec2<f32>, angle: f32, zoom: f32) -> vec2<f32> {
    let d = (uv - vec2<f32>(0.5)) * frame.size - offset * frame.size.y;
    let turn = vec2<f32>(cos(angle), sin(angle));
    let back = vec2<f32>(turn.x * d.x + turn.y * d.y, turn.x * d.y - turn.y * d.x) / zoom;
    return fold(back / frame.size + vec2<f32>(0.5));
}

// How much the frame must be enlarged so a move of `reach` frame heights
// and a turn of `tilt` radians never shows its edge.
fn cover(reach: f32, tilt: f32) -> f32 {
    let wide = max(frame.size.x / frame.size.y, frame.size.y / frame.size.x);
    let turned = cos(tilt) + sin(tilt) * wide;
    return turned * (1.0 + 2.0 * reach);
}

// Smooth noise in -1..1: a value at every whole number of `t`, eased
// between with a quintic so its rate is smooth too.
fn wobble(t: f32, seed: f32) -> f32 {
    let i = floor(t);
    let f = t - i;
    let a = hash(vec2<f32>(i, seed), 5.0) * 2.0 - 1.0;
    let b = hash(vec2<f32>(i + 1.0, seed), 5.0) * 2.0 - 1.0;
    return mix(a, b, f * f * f * (f * (f * 6.0 - 15.0) + 10.0));
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let amount = clamp(params.amount, 0.0, 100.0) / 50.0;
    let split = clamp(params.split, 0.0, 100.0) / 100.0;
    let rate = 12.0 * exp2((clamp(params.speed, 0.0, 100.0) - 50.0) / 25.0);
    let t = frame.time * rate;
    let shared_move = vec2<f32>(wobble(t, 1.0), wobble(t, 2.0)) * 0.008 * amount;
    let reach = 0.008 * amount + 0.022 * amount * split;
    let zoom = cover(reach, 0.0);
    let r = shared_move + vec2<f32>(wobble(t, 3.0), wobble(t, 4.0)) * 0.022 * amount * split;
    let b = shared_move + vec2<f32>(wobble(t, 5.0), wobble(t, 6.0)) * 0.022 * amount * split;
    let red = sample(through(uv, r, 0.0, zoom));
    let green = sample(through(uv, shared_move, 0.0, zoom));
    let blue = sample(through(uv, b, 0.0, zoom));
    return vec4<f32>(red.r, green.g, blue.b, green.a);
}
