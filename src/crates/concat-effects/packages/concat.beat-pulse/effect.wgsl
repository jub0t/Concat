// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Beat Pulse: a lift of light on every beat from the start of the clip,
// up to two stops, falling away over its share of the beat. The highlights
// soften at the top of each hit so a bright sky blooms rather than turns
// into a flat sheet.

struct Params { bpm: f32, amount: f32, length: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let a = clamp(params.amount, 0.0, 100.0) / 100.0;
    let beat = 60.0 / clamp(params.bpm, 40.0, 240.0);
    let t = max(frame.clip_time, 0.0);
    let since = t - floor(t / beat) * beat;
    let hold = beat * clamp(params.length, 10.0, 100.0) / 100.0;
    let q = clamp(since / hold, 0.0, 1.0);
    let e = (1.0 - q) * (1.0 - q) * a;
    let lit = exposure(c.rgb, 2.0 * e);
    // A soft shoulder over white, gentler the less the hit.
    let knee = lit / (vec3<f32>(1.0) + max(lit - vec3<f32>(1.0), vec3<f32>(0.0)) * e);
    return vec4<f32>(knee, c.a);
}
