struct Params { strength: f32 }

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let base = mix(from_at(uv), to_at(uv), progress);
    let flash_curve = 1.0 - abs(progress * 2.0 - 1.0);
    let leak_pos = uv.x * 0.7 + uv.y * 0.3;
    let leak = exp(-pow(leak_pos - progress, 2.0) * 12.0);
    let burn = (flash_curve * 0.8 + leak * 0.5) * (params.strength * 0.01);
    let warm_flash = vec3<f32>(1.0, 0.85, 0.6) * burn;
    return vec4<f32>(clamp01(base.rgb + warm_flash), base.a);
}
