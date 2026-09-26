struct Params { amount: f32 }

// Contrast in stops about middle grey (see `contrast`): a push takes every
// level further from grey by the same share of its distance in stops, a
// pull draws them together, and at -100 the picture is flat grey. What the
// Adjust panel's contrast does, alone: the slider's percent is the share.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    return vec4<f32>(contrast(c.rgb, 1.0 + params.amount / 100.0), c.a);
}
