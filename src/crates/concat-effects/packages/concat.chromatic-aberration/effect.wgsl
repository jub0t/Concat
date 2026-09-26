struct Params { shift: f32 }

// Lateral chromatic aberration: the red image a little larger than the
// green and the blue a little smaller, about the middle, so the fringes
// grow towards the edges - `shift` pixels at the edge of the picture's
// width - in light.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let scale = params.shift / (0.5 * frame.size.x);
    let from_middle = uv - vec2<f32>(0.5);
    let red = sample(vec2<f32>(0.5) + from_middle / (1.0 + scale));
    let blue = sample(vec2<f32>(0.5) + from_middle / max(1.0 - scale, 0.01));
    return vec4<f32>(red.r, c.g, blue.b, c.a);
}
