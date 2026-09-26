fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let g = hsl_band(c.rgb, 120.0, 60.0, 0.0, 1.35, 1.05);
    let b = hsl_band(g, 210.0, 50.0, 0.0, 1.25, 1.0);
    return vec4<f32>(contrast(b, 1.12), c.a);
}
