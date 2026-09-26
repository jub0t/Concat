struct Params {
    luma: array<vec4<f32>, 8>,
    red: array<vec4<f32>, 8>,
    green: array<vec4<f32>, 8>,
    blue: array<vec4<f32>, 8>,
}

// Curves on the display level (see `grade_curves`): the luma curve moves a
// level's brightness and keeps its colour, then red, green and blue each
// follow their own; a level past white is carried on past the curve's
// white. What the Adjust panel's curves do, alone.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    return vec4<f32>(grade_curves(c.rgb, params.luma, params.red, params.green, params.blue), c.a);
}
