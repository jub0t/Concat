fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let s = saturation(c.rgb, 0.35);
    let f = film_curve(s, 0.15, 0.25);
    return vec4<f32>(contrast(f, 1.25), c.a);
}
