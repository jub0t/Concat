// The left half, and its reflection on the right.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let x = select(1.0 - uv.x, uv.x, uv.x < 0.5);
    return sample(vec2<f32>(x, uv.y));
}
