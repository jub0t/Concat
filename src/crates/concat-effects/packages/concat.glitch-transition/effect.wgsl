struct Params { amount: f32 }

// Horizontal slices of the outgoing picture jitter sideways and split their
// channels; the tearing peaks at the middle of the cut.
fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let k = (1.0 - abs(progress * 2.0 - 1.0)) * params.amount * 0.01;
    let slice = floor(uv.y * 24.0);
    let jitter = (hash(vec2<f32>(slice, 0.0), floor(progress * 40.0)) - 0.5) * k * 0.15;
    let ux = clamp(uv.x + jitter, 0.0, 1.0);
    let a = vec4<f32>(
        from_at(vec2<f32>(clamp(ux + k * 0.01, 0.0, 1.0), uv.y)).r,
        from_at(vec2<f32>(ux, uv.y)).g,
        from_at(vec2<f32>(clamp(ux - k * 0.01, 0.0, 1.0), uv.y)).b,
        1.0,
    );
    let b = to_at(vec2<f32>(clamp(uv.x - jitter, 0.0, 1.0), uv.y));
    return mix(a, b, smoothstep(0.0, 1.0, progress));
}
