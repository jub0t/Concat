struct Params { temperature: f32 }

// Colour temperature as a white balance shift: the picture as if it were
// lit at `temperature` kelvin while the camera was set for daylight.

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    return vec4<f32>(clamp01(white_balance(c.rgb, params.temperature)), c.a);
}
