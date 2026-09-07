struct Params { amount: f32 }

// Green in the middle and the light, colour down, a light matte; skin held.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let a = params.amount / 100.0;
    let keep = 1.0 - skin_mask(c.rgb) * 0.5;
    var out = matte(c.rgb, a * 0.06, 1.0 - a * 0.03);
    out = saturation(out, 1.0 - a * 0.25);
    out = tint_midtones(out, vec3<f32>(-0.03, 0.08, 0.0), a * keep);
    out = split_tone(out, vec3<f32>(0.0), vec3<f32>(0.0, 0.06, -0.03), a);
    return vec4<f32>(clamp01(out), c.a);
}
