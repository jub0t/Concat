struct Params { contrast: f32 }

// Contrast about middle grey, with the ends rolled off by a film curve
// rather than clipped, so a pop never crushes a face into the floor.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let amount = params.contrast - 1.0;
    var out = contrast(c.rgb, 1.0 + amount * 0.7);
    out = film_curve(out, amount * 0.5, amount * 0.5);
    return vec4<f32>(clamp01(out), c.a);
}
