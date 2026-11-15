// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Push: the two shots sit edge to edge on one strip that slides a whole
// frame. The strip eases off and settles, and while it is moving fast the
// picture is smeared along the way it travels, as a camera would see it.

struct Params { direction: f32 }

// How far, in frames, the strip has moved by `p`: smoothstep's rate, so it
// leaves and lands with no jolt.
fn travelled(p: f32) -> f32 {
    return p * p * (3.0 - 2.0 * p);
}

// The rate of `travelled`: frames per whole cut.
fn pace(p: f32) -> f32 {
    return 6.0 * p * (1.0 - p);
}

// The way the strip moves on screen, one frame long.
fn heading(choice: f32) -> vec2<f32> {
    let pick = i32(round(clamp(choice, 0.0, 3.0)));
    var way = vec2<f32>(-1.0, 0.0);
    if (pick == 1) {
        way = vec2<f32>(1.0, 0.0);
    } else if (pick == 2) {
        way = vec2<f32>(0.0, -1.0);
    } else if (pick == 3) {
        way = vec2<f32>(0.0, 1.0);
    }
    return way;
}

// The strip at one instant, premultiplied: the last shot moved on by
// `t` frames, the next one a frame behind it.
fn strip(uv: vec2<f32>, way: vec2<f32>, t: f32) -> vec4<f32> {
    let next = uv + way * (1.0 - t);
    let inside = all(next >= vec2<f32>(0.0)) && all(next <= vec2<f32>(1.0));
    var c = from_at(uv - way * t);
    if (inside) {
        c = to_at(next);
    }
    return vec4<f32>(c.rgb * c.a, c.a);
}

// Taps along the motion, weighted to a tent so the smear has no hard
// ends; at full speed it is three hundredths of the frame.
const TAPS: i32 = 13;
const SMEAR: f32 = 0.03;

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = clamp(progress, 0.0, 1.0);
    let way = heading(params.direction);
    let t = travelled(p);
    let span = SMEAR * pace(p) / 1.5;
    var sum = vec4<f32>(0.0);
    var weight = 0.0;
    for (var k = 0; k < TAPS; k++) {
        let along = f32(k) / f32(TAPS - 1) - 0.5;
        let w = 1.0 - abs(along) * 1.8;
        sum += strip(uv - way * along * span, way, t) * w;
        weight += w;
    }
    let mean = sum / weight;
    return vec4<f32>(mean.rgb / max(mean.a, 1e-6), mean.a);
}
