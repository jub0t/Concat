struct Params { temperature: f32 }

// White balance toward candlelight, brightness held.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    return vec4<f32>(clamp01(white_balance(c.rgb, params.temperature)), c.a);
}
