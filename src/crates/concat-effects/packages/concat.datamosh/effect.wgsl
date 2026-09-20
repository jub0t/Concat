struct Params { amount: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let seedt = floor(frame.time * 8.0);
    let block = floor(uv * vec2<f32>(16.0, 9.0));
    let jitter = hash(block, seedt);
    let k = params.amount * 0.01;
    var offset = vec2<f32>(0.0);
    if (jitter > 1.0 - k * 0.6) {
        offset = vec2<f32>((hash(block, seedt + 1.0) - 0.5) * 0.06, 0.0);
    }
    return sample(uv + offset);
}
