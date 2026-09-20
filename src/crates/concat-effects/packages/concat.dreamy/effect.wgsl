fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let s = soften(uv, 3.0);
    let f = fade(mix(c.rgb, s, 0.4), 0.08);
    return vec4<f32>(clamp01(saturation(f, 0.9)), c.a);
}
