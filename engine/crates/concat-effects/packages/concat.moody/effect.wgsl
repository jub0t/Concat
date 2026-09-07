struct Params { depth: f32, warmth: f32 }

// A toe, drained colour, bronze in the middle, a cool floor, and the corners.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let depth = params.depth / 100.0;
    var out = film_curve(c.rgb, depth * 0.9, 0.0) * (1.0 - depth * 0.06);
    out = saturation(out, 1.0 - depth * 0.4);
    out = tint_midtones(out, vec3<f32>(0.1, 0.03, -0.08), params.warmth / 100.0);
    out = split_tone(out, vec3<f32>(0.0, 0.0, 0.06), vec3<f32>(0.0), depth);
    out = vignette(out, uv, depth * 0.7);
    return vec4<f32>(clamp01(out), c.a);
}
