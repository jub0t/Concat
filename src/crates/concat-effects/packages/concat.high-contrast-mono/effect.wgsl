fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let g = mono(c.rgb, vec3<f32>(0.33));
    return vec4<f32>(contrast(g, 1.6) - vec3<f32>(0.03), c.a);
}
