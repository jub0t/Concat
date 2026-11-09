// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Beat Shake: the Impact hit, repeated on every beat of a tempo from the
// start of the clip or layer - put it where the first beat falls. Each
// beat is knocked its own way; a hit lasts its share of the beat.

struct Params { bpm: f32, amount: f32, length: f32, punch: f32, blur: f32 }

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

fn camera(uv: vec2<f32>, shutter: f32) -> vec2<f32> {
    let beat = 60.0 / clamp(params.bpm, 40.0, 240.0);
    let t = max(frame.clip_time + shutter, 0.0);
    let n = floor(t / beat);
    let since = t - n * beat;
    let length = beat * clamp(params.length, 10.0, 100.0) / 100.0;
    let amount = clamp(params.amount, 0.0, 100.0) / 100.0;
    let decay = exp(-3.0 * since / length);
    let ring = cos(6.28318531 * 2.0 * since / length * 2.0);
    let a = hash(vec2<f32>(n, 11.0), 3.0) * 6.28318531;
    let offset = vec2<f32>(cos(a), sin(a)) * 0.045 * amount * decay * ring;
    let angle = 0.02 * amount * decay * ring * select(1.0, -1.0, hash(vec2<f32>(n, 13.0), 3.0) > 0.5);
    let zoom = 1.0 + 0.2 * clamp(params.punch, 0.0, 100.0) / 100.0 * exp(-4.0 * since / length);
    return through(uv, offset, angle, zoom * cover(0.045 * amount, 0.02 * amount));
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
