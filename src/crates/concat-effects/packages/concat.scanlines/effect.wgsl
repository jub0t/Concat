struct Params { strength: f32, pitch: f32 }

// One row in every `pitch` is darkened by the strength.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let pitch = max(round(params.pitch), 2.0);
    let row = floor(uv.y * frame.size.y);
    let dark = select(0.0, params.strength / 100.0, (row % pitch) < 1.0);
    return vec4<f32>(c.rgb * (1.0 - dark), c.a);
}
