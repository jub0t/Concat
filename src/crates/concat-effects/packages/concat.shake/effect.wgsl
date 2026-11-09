// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Shake: the camera moves on smooth noise - a slow drift by hand, a fast
// small jitter, or a quake that is both big and quick - and turns a little
// with it. The frame is enlarged just enough that no move shows an edge,
// and the shutter smears it as it goes.

struct Params { style: f32, amount: f32, speed: f32, rotation: f32, blur: f32 }

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

// A style's reach (frame heights), turn (radians) and rate (moves a
// second) at an amount of 50%, rotation of 50% and speed of 50%.
fn style() -> vec3<f32> {
    let pick = i32(round(clamp(params.style, 0.0, 2.0)));
    if (pick == 1) {
        return vec3<f32>(0.008, 0.006, 14.0);
    }
    if (pick == 2) {
        return vec3<f32>(0.035, 0.022, 7.0);
    }
    return vec3<f32>(0.016, 0.014, 1.4);
}

fn camera(uv: vec2<f32>, shutter: f32) -> vec2<f32> {
    let s = style();
    let amount = clamp(params.amount, 0.0, 100.0) / 50.0;
    let reach = s.x * amount;
    let tilt = s.y * amount * clamp(params.rotation, 0.0, 100.0) / 50.0;
    let rate = s.z * exp2((clamp(params.speed, 0.0, 100.0) - 50.0) / 25.0);
    let t = (frame.time + shutter) * rate;
    // Two octaves: the move and a finer tremble on it.
    let offset = vec2<f32>(
        wobble(t, 1.0) * 0.75 + wobble(t * 2.3, 2.0) * 0.25,
        wobble(t, 3.0) * 0.75 + wobble(t * 2.3, 4.0) * 0.25
    ) * reach;
    let angle = (wobble(t * 0.8, 5.0) * 0.8 + wobble(t * 1.9, 6.0) * 0.2) * tilt;
    return through(uv, offset, angle, cover(reach, tilt));
}

// Instants the shutter is open for, averaged: a fast move smears along
// its own path, a slow one stays sharp.
const TAPS: i32 = 9;

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let open = clamp(params.blur, 0.0, 100.0) / 100.0 / 24.0;
    var gathered = vec4<f32>(0.0);
    for (var k = 0; k < TAPS; k++) {
        let when = (f32(k) / f32(TAPS - 1) - 0.5) * open;
        gathered += sample_premultiplied(camera(uv, when));
    }
    let mean = gathered / f32(TAPS);
    return vec4<f32>(mean.rgb / max(mean.a, 1e-6), mean.a);
}
