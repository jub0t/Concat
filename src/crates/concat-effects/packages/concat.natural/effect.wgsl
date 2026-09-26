fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let wb = white_balance(c.rgb, 5600.0);
    return vec4<f32>(contrast(saturation(wb, 1.05), 1.04), c.a);
}
