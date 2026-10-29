struct Params { scale: f32 }

// Zoom through: push into the outgoing shot, then pull back out of the
// incoming one. At the middle of the cut both are at the maximum zoom.
fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = smoothstep(0.0, 1.0, progress);
    // Zoom factor: up at the middle of the cut, down at either end.
    let z = 1.0 + (params.scale - 1.0) * 4.0 * p * (1.0 - p);
    let center = vec2<f32>(0.5, 0.5);
    // First half: zoom into outgoing; second half: zoom out from incoming.
    let (coord, col) = if p < 0.5 {
        let c = center + (uv - center) * z;
        (c, from_at(c))
    } else {
        let c = center + (uv - center) / z;
        (c, to_at(c))
    };
    if (coord.x >= 0.0 && coord.x <= 1.0 && coord.y >= 0.0 && coord.y <= 1.0) {
        return col;
    }
    // Outside the zoomed frame: show the edge colour.
    if (p < 0.5) {
        return from_at(uv);
    }
    return to_at(uv);
}
