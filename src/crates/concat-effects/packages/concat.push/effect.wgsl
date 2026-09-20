struct Params { direction: f32 }

fn axis(direction: f32) -> vec2<f32> {
    if (direction < 0.5) {
        return vec2<f32>(1.0, 0.0);
    } else if (direction < 1.5) {
        return vec2<f32>(-1.0, 0.0);
    } else if (direction < 2.5) {
        return vec2<f32>(0.0, 1.0);
    }
    return vec2<f32>(0.0, -1.0);
}

// Both layers move together along the same axis: the outgoing one is
// carried off as the incoming one is carried into place.
fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = smoothstep(0.0, 1.0, progress);
    let dir = axis(params.direction);
    let to_uv = uv - dir * (1.0 - p);
    if (to_uv.x >= 0.0 && to_uv.x <= 1.0 && to_uv.y >= 0.0 && to_uv.y <= 1.0) {
        return to_at(to_uv);
    }
    let from_uv = uv + dir * p;
    return from_at(from_uv);
}
