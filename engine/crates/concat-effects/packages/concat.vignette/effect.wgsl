struct Params { strength: f32 }

// Darkens towards the corners: a smooth fall-off from the centre.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let d = length((uv - vec2<f32>(0.5)) * vec2<f32>(2.0));
    let s = params.strength / 100.0;
    let fall = smoothstep(1.4 - s * 0.9, 1.4 + 0.2 - s * 0.3, d);
    return vec4<f32>(c.rgb * (1.0 - fall * (0.4 + 0.6 * s)), c.a);
}
