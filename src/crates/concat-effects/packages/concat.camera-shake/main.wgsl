struct Params { amplitude: f32, speed: f32, rotation: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let t = frame.time * params.speed;
    let amp = params.amplitude * texel() * 0.5;
    let offset = vec2<f32>(sin(t * 1.1) + 0.5 * sin(t * 2.3), cos(t * 0.9) + 0.5 * cos(t * 2.7)) * amp;
    let angle = (sin(t * 0.7) + 0.5 * cos(t * 1.7)) * (params.rotation * 0.01745329);
    let center = vec2<f32>(0.5, 0.5);
    let d = uv + offset - center;
    let cos_a = cos(angle);
    let sin_a = sin(angle);
    let rotated = vec2<f32>(d.x * cos_a - d.y * sin_a, d.x * sin_a + d.y * cos_a);
    return sample(center + rotated);
}
