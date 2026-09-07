struct Params { amount: f32, contrast: f32, skin: f32 }

// The split tone held back on skin, then a film curve rather than a hard one.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let keep = 1.0 - skin_mask(c.rgb) * params.skin / 100.0;
    var out = split_tone(c.rgb, vec3<f32>(-0.12, 0.04, 0.16), vec3<f32>(0.16, 0.04, -0.12), params.amount / 100.0 * keep);
    out = film_curve(out, params.contrast / 100.0 * 0.6, params.contrast / 100.0 * 0.5);
    return vec4<f32>(clamp01(out), c.a);
}
