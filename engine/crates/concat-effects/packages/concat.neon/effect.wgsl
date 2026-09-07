struct Params { glow: f32, punch: f32 }

// The same split tone as a film grade, with the two ends swapped for signage.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let punch = params.punch / 100.0;
    var out = split_tone(c.rgb, vec3<f32>(0.18, -0.1, 0.18), vec3<f32>(-0.1, 0.1, 0.15), params.glow / 100.0);
    out = contrast(out, 1.0 + punch * 0.5);
    out = saturation(out, 1.0 + punch * 0.6);
    return vec4<f32>(clamp01(out), c.a);
}
