fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let t = split_tone(c.rgb, vec3<f32>(0.08, -0.02, 0.12), vec3<f32>(-0.05, 0.08, 0.1), 1.0);
    return vec4<f32>(contrast(saturation(t, 1.15), 1.15), c.a);
}
