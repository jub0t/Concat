struct Params { direction: f32, ease: f32 }

// Drag: both layers move together along the same axis, the outgoing one
// carried off as the incoming one is carried into place. The ease knob
// softens the start and end of the move.
fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = smoothstep(0.0, 1.0, progress);
    // Ease: soften the move at both ends.
    let e = p * p * (3.0 - 2.0 * p);
    let pe = mix(p, e, params.ease);
    var d: vec2<f32>;
    if (params.direction < 0.5) {
        d = vec2<f32>(1.0, 0.0);
    } else if (params.direction < 1.5) {
        d = vec2<f32>(-1.0, 0.0);
    } else if (params.direction < 2.5) {
        d = vec2<f32>(0.0, 1.0);
    } else {
        d = vec2<f32>(0.0, -1.0);
    }
    let to_uv = uv - d * (1.0 - pe);
    if (to_uv.x >= 0.0 && to_uv.x <= 1.0 && to_uv.y >= 0.0 && to_uv.y <= 1.0) {
        return to_at(to_uv);
    }
    let from_uv = uv + d * pe;
    return from_at(from_uv);
}
