// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Impact: the jolt of something landing, timed from the start of the clip
// or layer it is on. The frame is knocked one way and rings back and forth
// like a spring, each swing smaller, while a punch of zoom in falls away;
// by the end of its length it has settled.

struct Params { amount: f32, length: f32, punch: f32, direction: f32, blur: f32 }

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

// One hit's camera at `t` seconds after it, as an offset (frame heights),
// a turn and a zoom; `seed` picks which way it is knocked.
fn hit(t: f32, length: f32, seed: f32) -> vec4<f32> {
    let amount = clamp(params.amount, 0.0, 100.0) / 100.0;
    // Gone to a twentieth by the end of its length.
    let decay = exp(-3.0 * max(t, 0.0) / length);
    let ring = cos(6.28318531 * 2.2 * max(t, 0.0) / length * 3.0);
    let a = hash(vec2<f32>(seed, 7.0), 3.0) * 6.28318531;
    var way = vec2<f32>(cos(a), sin(a));
    let pick = i32(round(clamp(params.direction, 0.0, 2.0)));
    if (pick == 1) {
        way = vec2<f32>(select(-1.0, 1.0, way.x >= 0.0), 0.0);
    } else if (pick == 2) {
        way = vec2<f32>(0.0, select(-1.0, 1.0, way.y >= 0.0));
    }
    let offset = way * 0.06 * amount * decay * ring;
    let angle = 0.03 * amount * decay * sin(6.28318531 * 1.6 * max(t, 0.0) / length * 3.0)
        * select(1.0, -1.0, hash(vec2<f32>(seed, 9.0), 3.0) > 0.5);
    let zoom = 1.0 + 0.25 * clamp(params.punch, 0.0, 100.0) / 100.0 * exp(-4.0 * max(t, 0.0) / length);
    return vec4<f32>(offset, angle, zoom);
}

fn camera(uv: vec2<f32>, shutter: f32) -> vec2<f32> {
    let length = clamp(params.length, 0.1, 2.0);
    let h = hit(frame.clip_time + shutter, length, 0.0);
    let amount = clamp(params.amount, 0.0, 100.0) / 100.0;
    return through(uv, h.xy, h.z, h.w * cover(0.06 * amount, 0.03 * amount));
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
