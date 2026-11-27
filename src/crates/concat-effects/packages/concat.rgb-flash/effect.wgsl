// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// RGB Flash: hits at a steady speed from the start of the clip. On each
// one red and blue are thrown apart along a direction of their own - a
// fresh one every hit - and the light jumps up, then both fall back to
// the picture before the next.

struct Params { amount: f32, speed: f32, split: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let a = clamp(params.amount, 0.0, 100.0) / 100.0;
    let t = max(frame.clip_time, 0.0) * clamp(params.speed, 0.5, 8.0);
    let n = floor(t);
    let q = t - n;
    let e = exp(-6.0 * q) * a;
    let turn = hash(vec2<f32>(n, 5.0), 11.0) * 6.28318531;
    let reach = 0.05 * clamp(params.split, 0.0, 100.0) / 100.0 * e;
    let way = vec2<f32>(cos(turn), sin(turn) * frame.size.x / frame.size.y) * reach;
    let g = sample(uv);
    let r = sample(uv + way).r;
    let b = sample(uv - way).b;
    let lit = exposure(vec3<f32>(r, g.g, b), 1.5 * e);
    return vec4<f32>(lit, g.a);
}
