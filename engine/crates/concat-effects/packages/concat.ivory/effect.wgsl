struct Params { matte: f32, warmth: f32 }

// The matte, then cream in the light and a little gold through the middle.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let m = params.matte / 100.0;
    let w = params.warmth / 100.0;
    var out = lift_gamma_gain(c.rgb, vec3<f32>(0.0), vec3<f32>(1.0 + m * 0.18), vec3<f32>(1.0));
    out = matte(out, m * 0.12, 1.0 - m * 0.04);
    out = saturation(out, 1.0 - m * 0.18);
    out = split_tone(out, vec3<f32>(0.0), vec3<f32>(0.08, 0.03, -0.06), w);
    out = tint_midtones(out, vec3<f32>(0.04, 0.0, -0.04), w);
    return vec4<f32>(clamp01(out), c.a);
}
