fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let g = mono(c.rgb, vec3<f32>(0.3, 0.33, 0.37));
    let f = fade(g, 0.05);
    let t = tint_midtones(f, vec3<f32>(-0.02, 0.0, 0.03), 1.0);
    return vec4<f32>(t, c.a);
}
