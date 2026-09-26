struct Params { amount: f32 }

// Grain in log, where film's is: every pixel moved up or down by the same
// share of a stop whatever its level, so a highlight grains like a shadow,
// and a level past white stays past it. New every frame, and the same on
// every GPU for the same frame (see `hash`).
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let n = hash(floor(uv * frame.size), fract(frame.time * 7.31)) - 0.5;
    return vec4<f32>(from_log(to_log(c.rgb) + vec3<f32>(n * params.amount * 0.008)), c.a);
}
