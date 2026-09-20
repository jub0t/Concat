struct Params { amount: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let e = edge_at(uv);
    let hue = fract(uv.x + uv.y + frame.time * 0.05);
    let neon = hue_rotate(vec3<f32>(0.2, 1.0, 1.0), hue * 360.0);
    let glow = e * params.amount * 0.01;
    return vec4<f32>(clamp01(neon * glow), 1.0);
}
