struct Params { haze: f32, chill: f32 }

// A matte, colour drained, cool through the dark, the brights blooming.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let h = params.haze / 100.0;
    let ch = params.chill / 100.0;
    var out = matte(c.rgb, h * 0.08, 1.0 - h * 0.02);
    out = saturation(out, 1.0 - h * 0.4);
    out = contrast(out, 1.0 - h * 0.08);
    out = split_tone(out, vec3<f32>(0.0, 0.0, 0.1), vec3<f32>(0.0), ch);
    out = tint_midtones(out, vec3<f32>(0.0, 0.0, 0.06), ch);
    out = halation(uv, out, 0.65, 12.0, vec3<f32>(0.85, 0.92, 1.0), h * 0.45);
    return vec4<f32>(clamp01(out), c.a);
}
