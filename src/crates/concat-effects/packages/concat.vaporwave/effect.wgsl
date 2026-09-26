fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let t = split_tone(c.rgb, vec3<f32>(0.1, 0.0, 0.15), vec3<f32>(0.0, 0.08, 0.12), 1.0);
    return vec4<f32>(saturation(t, 1.3), c.a);
}
