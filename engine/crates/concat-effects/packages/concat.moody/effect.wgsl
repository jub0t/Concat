struct Params { depth: f32, warmth: f32 }

// Darker, harder, drained, and bronze through the middle.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let depth = params.depth / 100.0;
    var out = c.rgb - vec3<f32>(depth * 0.06);
    out = contrast(out, 1.0 + depth * 0.35);
    out = saturation(out, 1.0 - depth * 0.4);
    out = tint_midtones(out, vec3<f32>(0.1, 0.03, -0.08), params.warmth / 100.0);
    out = vignette(out, uv, depth * 0.7);
    return vec4<f32>(clamp01(out), c.a);
}
