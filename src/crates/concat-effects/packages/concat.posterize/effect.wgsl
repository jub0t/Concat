struct Params { levels: f32 }

// Flat bands on the display level: each channel rounded to one of `levels`
// steps from black to white, white among them, and on at the same spacing
// past it.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let steps = max(round(params.levels), 2.0) - 1.0;
    return vec4<f32>(round(c.rgb * steps) / steps, c.a);
}
