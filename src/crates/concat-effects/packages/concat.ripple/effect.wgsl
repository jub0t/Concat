struct Params { amount: f32, speed: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = uv - vec2<f32>(0.5);
    let d = length(c);
    let wave = sin(d * 40.0 - frame.time * params.speed) * params.amount * 0.001;
    let dir = c / max(d, 0.0001);
    return sample(uv + dir * wave);
}
