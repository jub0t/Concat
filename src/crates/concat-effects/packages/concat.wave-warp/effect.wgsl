struct Params { amplitude: f32, wavelength: f32, speed: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let t = frame.time * params.speed;
    let wave_x = sin(uv.y * (params.wavelength * 0.5) + t) * (params.amplitude * 0.001);
    let wave_y = cos(uv.x * (params.wavelength * 0.5) + t) * (params.amplitude * 0.001);
    return sample(uv + vec2<f32>(wave_x, wave_y));
}
