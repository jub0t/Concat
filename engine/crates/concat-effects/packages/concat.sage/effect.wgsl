struct Params { amount: f32 }

// Colour down, the middle turned green-teal except on skin, a soft curve.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let a = params.amount / 100.0;
    let keep = 1.0 - skin_mask(c.rgb) * 0.8;
    var out = saturation(c.rgb, 1.0 - a * 0.3);
    out = tint_midtones(out, vec3<f32>(-0.04, 0.06, 0.04), a * keep);
    out = matte(film_curve(out, 0.3 * a, 0.3 * a), 0.02 * a, 1.0 - 0.02 * a);
    return vec4<f32>(clamp01(out), c.a);
}
