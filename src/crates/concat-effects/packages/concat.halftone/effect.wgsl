struct Params { size: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let cell = max(params.size, 2.0) * texel();
    let center = (floor(uv / cell) + vec2<f32>(0.5)) * cell;
    let l = luma(sample(center).rgb);
    let d = distance(uv, center) / (cell.x * 0.5 + 0.0001);
    let dot = step(d, 1.0 - l);
    return vec4<f32>(vec3<f32>(dot), c.a);
}
