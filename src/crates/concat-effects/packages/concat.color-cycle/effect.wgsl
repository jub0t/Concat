struct Params { speed: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let rad = frame.time * params.speed * 0.017453;
    let cos_a = cos(rad);
    let sin_a = sin(rad);
    let rot_r = c.r * (0.299 + 0.701 * cos_a + 0.168 * sin_a)
              + c.g * (0.587 - 0.587 * cos_a + 0.330 * sin_a)
              + c.b * (0.114 - 0.114 * cos_a - 0.497 * sin_a);
    let rot_g = c.r * (0.299 - 0.299 * cos_a - 0.328 * sin_a)
              + c.g * (0.587 + 0.413 * cos_a + 0.035 * sin_a)
              + c.b * (0.114 - 0.114 * cos_a + 0.288 * sin_a);
    let rot_b = c.r * (0.299 - 0.300 * cos_a - 1.250 * sin_a)
              + c.g * (0.587 - 0.588 * cos_a - 1.050 * sin_a)
              + c.b * (0.114 + 0.886 * cos_a - 0.203 * sin_a);
    return vec4<f32>(clamp01(vec3<f32>(rot_r, rot_g, rot_b)), c.a);
}
