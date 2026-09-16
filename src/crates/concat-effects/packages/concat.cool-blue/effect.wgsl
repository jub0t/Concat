struct Params { chill: f32, contrast: f32 }

// Blue where it is dark, not on faces, and a curve.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let chill = params.chill / 100.0 * (1.0 - skin_mask(c.rgb) * 0.7);
    var out = split_tone(c.rgb, vec3<f32>(-0.08, 0.0, 0.25), vec3<f32>(0.0), chill);
    out = tint_midtones(out, vec3<f32>(0.0, 0.0, 0.1), chill);
    out = s_curve(out, params.contrast / 100.0 * 0.7);
    return vec4<f32>(clamp01(out), c.a);
}
