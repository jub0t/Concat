struct Params { amount: f32 }

// Saturation about luminance, by the display level, as the Adjust panel's
// slider makes it: a push up to three times richer at +100, a pull to grey
// at -100, easing in so the first steps either way are gentle.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let s = params.amount / 100.0;
    return vec4<f32>(saturation(c.rgb, select((1.0 + s) * (1.0 + s), 1.0 + 2.0 * s, s > 0.0)), c.a);
}
