struct Params { colour: f32, contrast: f32 }

// The channel weights slide from a green filter to a red one.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let f = params.colour / 100.0;
    let r = 0.3 + 0.35 * f;
    let b = 0.11 - 0.05 * f;
    var out = mono(c.rgb, vec3<f32>(r, 1.0 - r - b, b));
    out = contrast(out, 1.0 + params.contrast / 100.0 * 0.4);
    return vec4<f32>(clamp01(out), c.a);
}
