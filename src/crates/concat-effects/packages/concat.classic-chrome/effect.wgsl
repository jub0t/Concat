fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let f = film_curve(c.rgb, 0.1, 0.35);
    return vec4<f32>(saturation(f, 0.9), c.a);
}
