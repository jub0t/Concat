struct Params { direction: f32, speed: f32 }

// Whip pan: the camera whips fast along an axis. Near the middle of the cut
// the picture blurs into a directional streak, then resolves into the
// incoming shot. The speed knob controls how fast the whip is.
fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = smoothstep(0.0, 1.0, progress);
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
    // The whip: at the middle of the cut, sample far along the whip axis.
    let whip = p * (1.0 - p) * 4.0 * params.speed;
    let coord = uv + d * whip;
    // Where the whip streak leaves the frame, cross-fade instead.
    let blend = smoothstep(0.0, 1.0, (coord.x - 0.5).abs() + (coord.y - 0.5).abs());
    let from_col = from_at(uv);
    let to_col = to_at(uv);
    mix(from_col, to_col, blend);
}
