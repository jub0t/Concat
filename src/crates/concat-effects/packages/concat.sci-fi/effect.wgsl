fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let wb = white_balance(c.rgb, 8200.0);
    let t = split_tone(wb, vec3<f32>(-0.02, 0.03, 0.06), vec3<f32>(0.0, 0.02, 0.03), 1.0);
    return vec4<f32>(saturation(t, 0.85), c.a);
}
