fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let wb = white_balance(c.rgb, 5200.0);
    let s = saturation(wb, 1.3);
    return vec4<f32>(s_curve(s, 0.25), c.a);
}
