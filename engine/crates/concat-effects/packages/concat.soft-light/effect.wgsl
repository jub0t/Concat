struct Params { lift: f32, warmth: f32 }

// Matte, less contrast, less colour, cream in the brights, a bloom over them.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let lift = params.lift / 100.0;
    var out = matte(c.rgb, lift * 0.1, 1.0 - lift * 0.04);
    out = contrast(out, 1.0 - lift * 0.18);
    out = saturation(out, 1.0 - lift * 0.12);
    out = split_tone(out, vec3<f32>(0.0), vec3<f32>(0.07, 0.02, -0.06), params.warmth / 100.0);
    out = halation(uv, out, 0.6, 10.0, vec3<f32>(1.0, 0.97, 0.9), lift * 0.35);
    return vec4<f32>(clamp01(out), c.a);
}
