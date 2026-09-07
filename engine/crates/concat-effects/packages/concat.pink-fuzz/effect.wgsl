struct Params { haze: f32, blush: f32 }

// A matte, less contrast, the blush in the brights, a pink bloom screened over.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let haze = params.haze / 100.0;
    var out = matte(c.rgb, haze * 0.1, 1.0);
    out = contrast(out, 1.0 - haze * 0.2);
    out = saturation(out, 1.0 - haze * 0.15);
    out = split_tone(out, vec3<f32>(0.0), vec3<f32>(0.1, -0.04, 0.08), params.blush / 100.0);
    out = halation(uv, out, 0.5, 12.0, vec3<f32>(1.0, 0.85, 0.95), haze * 0.55);
    return vec4<f32>(clamp01(out), c.a);
}
