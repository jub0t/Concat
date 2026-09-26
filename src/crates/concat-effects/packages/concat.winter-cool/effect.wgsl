fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let wb = white_balance(c.rgb, 8800.0);
    let f = fade(wb, 0.06);
    return vec4<f32>(saturation(f, 0.8), c.a);
}
