struct Params { shift: f32 }

// Red taken from a little to one side and blue from the other: the two ends
// of the spectrum land apart, as a lens that cannot bring them together
// leaves them.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let d = vec2<f32>(params.shift * texel().x, 0.0);
    let c = sample(uv);
    let r = sample(uv + d).r;
    let b = sample(uv - d).b;
    return vec4<f32>(r, c.g, b, c.a);
}
