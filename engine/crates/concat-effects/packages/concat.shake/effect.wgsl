struct Params { amount: f32, speed: f32 }

// The picture shifted by a wandering offset of up to `amount` pixels.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let t = frame.time;
    let dx = sin(t * params.speed) * params.amount;
    let dy = cos(t * params.speed * 1.3) * params.amount;
    let q = uv + vec2<f32>(dx, dy) / frame.size;
    if (q.x < 0.0 || q.x > 1.0 || q.y < 0.0 || q.y > 1.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    return sample(q);
}
