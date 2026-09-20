struct Params { direction: f32 }

// The incoming layer's on-screen offset: it starts fully off that edge and
// ends at rest. Wherever it has not yet reached, the outgoing layer shows
// through untouched underneath.
fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = smoothstep(0.0, 1.0, progress);
    var d: vec2<f32>;
    if (params.direction < 0.5) {
        d = vec2<f32>(-(1.0 - p), 0.0);
    } else if (params.direction < 1.5) {
        d = vec2<f32>(1.0 - p, 0.0);
    } else if (params.direction < 2.5) {
        d = vec2<f32>(0.0, -(1.0 - p));
    } else {
        d = vec2<f32>(0.0, 1.0 - p);
    }
    let coord = uv - d;
    if (coord.x < 0.0 || coord.x > 1.0 || coord.y < 0.0 || coord.y > 1.0) {
        return from_at(uv);
    }
    return to_at(coord);
}
