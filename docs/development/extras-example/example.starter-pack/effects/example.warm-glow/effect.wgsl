// SPDX-License-Identifier: CC0-1.0
// Warm Glow: red and a little green lifted, blue lowered, by the knob.

struct Params { warmth: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let w = params.warmth / 100.0;
    let tint = vec3<f32>(1.0 + 0.12 * w, 1.0 + 0.03 * w, 1.0 - 0.12 * w);
    return vec4<f32>(c.rgb * tint, c.a);
}
