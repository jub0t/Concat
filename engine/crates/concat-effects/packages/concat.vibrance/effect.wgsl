struct Params { amount: f32 }

// Saturates the dull colours more than the vivid ones, and skin less
// than either: a face should be the last thing a vibrance boost reaches.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let boosted = vibrance(c.rgb, params.amount);
    let out = mix(boosted, c.rgb, skin_mask(c.rgb) * 0.6);
    return vec4<f32>(clamp01(out), c.a);
}
