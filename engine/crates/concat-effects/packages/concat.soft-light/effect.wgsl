struct Params { lift: f32, warmth: f32 }

// A fade, less contrast, less colour, and cream in the brights.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let lift = params.lift / 100.0;
    var out = fade(c.rgb, lift * 0.12);
    out = contrast(out, 1.0 - lift * 0.2);
    out = saturation(out, 1.0 - lift * 0.15);
    out = split_tone(out, vec3<f32>(0.0), vec3<f32>(0.08, 0.02, -0.06), params.warmth / 100.0);
    return vec4<f32>(clamp01(out), c.a);
}
