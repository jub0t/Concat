struct Params { softness: f32 }

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = smoothstep(0.0, 1.0, progress);
    // A circle in pixels, measured in half-diagonals so it opens all the
    // way to the corners of any frame.
    let d = length((uv - vec2<f32>(0.5)) * frame.size) / (0.5 * length(frame.size));
    let radius = p * 1.15;
    let soft = max(params.softness * 0.01, 0.02);
    let m = smoothstep(radius - soft, radius + soft, d);
    return mix(to_at(uv), from_at(uv), m);
}
