fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let f = fade(c.rgb, 0.08);
    let t = split_tone(f, vec3<f32>(0.0), vec3<f32>(0.08, 0.0, -0.06), 1.0);
    let g = grain_at(uv, 0.05);
    return vec4<f32>(t + g, c.a);
}
