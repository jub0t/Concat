struct Params { amount: f32, speed: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let pulse = 0.6 + 0.4 * sin(frame.time * params.speed);
    let glow = halation(uv, c.rgb, 0.55, 12.0, vec3<f32>(1.0), params.amount * 0.01 * pulse);
    return vec4<f32>(glow, c.a);
}
