fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    return vec4<f32>(vec3<f32>(1.0) - c.rgb, c.a);
}
