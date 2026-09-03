struct Params { amount: f32 }

// Per-pixel noise that changes every frame, scaled by the amount.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let n = hash(uv * frame.size, fract(frame.time * 7.31)) - 0.5;
    let grain = n * params.amount / 100.0;
    return vec4<f32>(clamp(c.rgb + vec3<f32>(grain), vec3<f32>(0.0), vec3<f32>(1.0)), c.a);
}
