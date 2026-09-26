fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let f = fade(c.rgb, 0.1);
    let t = tint_midtones(f, vec3<f32>(0.1, 0.06, -0.12), 1.0);
    return vec4<f32>(saturation(t, 0.85), c.a);
}
