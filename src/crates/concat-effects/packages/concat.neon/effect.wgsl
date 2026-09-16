struct Params { glow: f32, punch: f32 }

// The split tone with the ends swapped for signage, and the signs blooming.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let g = params.glow / 100.0;
    let punch = params.punch / 100.0;
    var out = split_tone(c.rgb, vec3<f32>(0.18, -0.1, 0.18), vec3<f32>(-0.1, 0.1, 0.15), g);
    out = contrast(out, 1.0 + punch * 0.5);
    out = saturation(out, 1.0 + punch * 0.6);
    out = halation(uv, out, 0.75, 10.0, vec3<f32>(1.0, 1.0, 1.0), g * 0.4);
    return vec4<f32>(clamp01(out), c.a);
}
