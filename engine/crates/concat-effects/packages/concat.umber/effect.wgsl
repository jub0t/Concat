struct Params { warmth: f32, soften: f32 }

// Brown through the midtones and shadows, greens to olive, a soft matte.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let w = params.warmth / 100.0;
    let s = params.soften / 100.0;
    var out = tint_midtones(c.rgb, vec3<f32>(0.1, 0.03, -0.1), w);
    out = split_tone(out, vec3<f32>(0.04, 0.0, -0.04), vec3<f32>(0.0), w);
    out = hsl_band(out, 110.0, 60.0, -20.0 * w, 1.0 - w * 0.3, 1.0 - w * 0.08);
    out = contrast(out, 1.0 - s * 0.12);
    out = matte(out, s * 0.05, 1.0);
    return vec4<f32>(clamp01(out), c.a);
}
