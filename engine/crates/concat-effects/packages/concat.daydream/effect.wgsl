struct Params { haze: f32, warmth: f32 }

// A matte, warm light blooming wide, a cool floor, skin untouched by the tone.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let haze = params.haze / 100.0;
    let w = params.warmth / 100.0;
    let keep = 1.0 - skin_mask(c.rgb) * 0.6;
    var out = matte(c.rgb, haze * 0.08, 1.0 - haze * 0.03);
    out = contrast(out, 1.0 - haze * 0.12);
    out = split_tone(out, vec3<f32>(0.0, 0.0, 0.04), vec3<f32>(0.08, 0.02, -0.05), w * keep);
    out = halation(uv, out, 0.55, 14.0, vec3<f32>(1.0, 0.9, 0.75), haze * 0.5);
    return vec4<f32>(clamp01(out), c.a);
}
