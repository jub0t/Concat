struct Params { amount: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = uv - vec2<f32>(0.5);
    let d = length(c) / 0.7071068;
    let k = params.amount * 0.01 * 0.5;
    let scale = 1.0 + k * (1.0 - min(d, 1.0));
    return sample(c * scale + vec2<f32>(0.5));
}
