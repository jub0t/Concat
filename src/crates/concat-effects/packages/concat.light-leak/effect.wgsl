struct Params { amount: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let sweep = uv.x + uv.y * 0.4 + sin(frame.time * 0.15) * 0.3 - 0.5;
    let leak = exp(-sweep * sweep * 8.0) * 1.2;
    let amber_tint = vec3<f32>(1.0, 0.72, 0.35);

    let edge_tl = min(uv.x, uv.y);
    let pulse_amber = sin(frame.time * 1.6) * 0.3 + 0.7;
    let amber_edge = exp(-edge_tl * 10.0) * pulse_amber * vec3<f32>(1.0, 0.55, 0.15);

    let edge_br = min(1.0 - uv.x, 1.0 - uv.y);
    let pulse_cyan = cos(frame.time * 1.3) * 0.3 + 0.7;
    let cyan_edge = exp(-edge_br * 12.0) * pulse_cyan * vec3<f32>(0.2, 0.85, 1.0);

    let total_burn = (amber_tint * leak + amber_edge + cyan_edge) * (params.amount * 0.01);
    return vec4<f32>(clamp01(c.rgb + total_burn), c.a);
}
