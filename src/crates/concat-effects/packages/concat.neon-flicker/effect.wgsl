struct Params { rate: f32, min_opacity: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let step_t = floor(frame.time * max(params.rate, 0.5));
    let on = hash(vec2<f32>(step_t, 0.0), 1.0);
    let flicker = mix(params.min_opacity * 0.01, 1.0, step(0.15, on));
    return vec4<f32>(c.rgb, c.a * flicker);
}
