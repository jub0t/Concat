fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let g = mono(c.rgb, vec3<f32>(0.3, 0.3, 0.4));
    let toned = tint_midtones(g, vec3<f32>(-0.02, 0.0, 0.05), 1.0);
    let v = vignette(toned, uv, 0.35);
    return vec4<f32>(contrast(v, 1.3), c.a);
}
