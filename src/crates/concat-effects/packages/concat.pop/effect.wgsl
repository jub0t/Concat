struct Params { scale: f32 }

// Pop in: the incoming shot scales from a small centre dot up to full size,
// with a slight overshoot bounce at the end. Where it has not yet reached,
// the outgoing shot shows through untouched.
fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = smoothstep(0.0, 1.0, progress);
    // A bounce: overshoot slightly then settle back to 1x.
    let bounce = 1.0 + 0.15 * sin(p * 6.28318 * 2.0) * (1.0 - p);
    let s = mix(0.25, 1.0, p) * params.scale * bounce;
    let center = vec2<f32>(0.5, 0.5);
    let coord = center + (uv - center) / s;
    if (coord.x >= 0.0 && coord.x <= 1.0 && coord.y >= 0.0 && coord.y <= 1.0) {
        return to_at(coord);
    }
    return from_at(uv);
}
