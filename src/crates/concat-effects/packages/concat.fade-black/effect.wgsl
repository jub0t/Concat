// Down to black over the first half of the cut, up out of it over the second;
// the two shots are never on screen together.
fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let black = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    if (progress < 0.5) {
        return mix(from_at(uv), black, progress * 2.0);
    }
    return mix(black, to_at(uv), (progress - 0.5) * 2.0);
}
