// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

// Flash: the picture blown out by a burst of light at the start of its
// clip, which fades on a curve that falls fast and lands softly. The
// light is added as a lamp would add it - the picture pushed up in stops
// - and at its height washed towards the flash's colour, so a white flash
// reads white and a red one red.

struct Params { amount: f32, length: f32, colour: vec4<f32> }

// The flash's strength `t` seconds in: whole at once, gone by `length`.
fn envelope(t: f32, length: f32) -> f32 {
    let q = clamp(t / length, 0.0, 1.0);
    let left = 1.0 - q;
    return left * left * left;
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let a = clamp(params.amount, 0.0, 100.0) / 100.0;
    let e = envelope(max(frame.clip_time, 0.0), clamp(params.length, 0.05, 2.0)) * a;
    let tint = from_display(clamp(params.colour.rgb, vec3<f32>(0.0), vec3<f32>(1.0)));
    let lit = exposure(c.rgb, 3.0 * e) * mix(vec3<f32>(1.0), tint, e);
    return vec4<f32>(mix(lit, tint, e * e * 0.8), c.a);
}
