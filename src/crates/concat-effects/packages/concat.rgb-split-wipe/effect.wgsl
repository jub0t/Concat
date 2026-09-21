struct Params { spread: f32 }

// The red and blue channels pull apart from green, peaking at the middle of
// the cut, on both the outgoing and incoming pictures.
fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let k = (1.0 - abs(progress * 2.0 - 1.0)) * params.spread * 0.0004;
    let a = vec4<f32>(
        from_at(vec2<f32>(uv.x + k, uv.y)).r,
        from_at(uv).g,
        from_at(vec2<f32>(uv.x - k, uv.y)).b,
        1.0,
    );
    let b = vec4<f32>(
        to_at(vec2<f32>(uv.x - k, uv.y)).r,
        to_at(uv).g,
        to_at(vec2<f32>(uv.x + k, uv.y)).b,
        1.0,
    );
    return mix(a, b, smoothstep(0.0, 1.0, progress));
}
