struct Params { strength: f32, frequency: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let flash_pulse = step(0.5, fract(frame.time * params.frequency * 0.5));
    let flash = flash_pulse * (params.strength * 0.01);
    let out_rgb = clamp01(mix(c.rgb, vec3<f32>(1.0), flash * 0.8));
    return vec4<f32>(out_rgb, c.a);
}
