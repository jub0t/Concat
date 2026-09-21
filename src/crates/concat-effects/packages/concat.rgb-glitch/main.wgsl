struct Params { amount: f32, frequency: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let t = floor(frame.time * params.frequency);
    let slice_y = floor(uv.y * 15.0);
    let slice_noise = hash(vec2<f32>(slice_y, t), 1.0);
    let is_glitch = step(0.65, slice_noise);
    let shift = (slice_noise - 0.5) * (params.amount * 0.001) * is_glitch;
    let r = sample(uv + vec2<f32>(shift * 2.0, 0.0)).r;
    let g = sample(uv + vec2<f32>(shift, 0.0)).g;
    let b = sample(uv - vec2<f32>(shift * 1.5, 0.0)).b;
    let a = sample(uv).a;
    return vec4<f32>(r, g, b, a);
}
