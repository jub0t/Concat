struct Params { tint: f32, speed: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let sweep = fract(uv.y * 1.5 - frame.time * params.speed * 0.1);
    let ramp = abs(sweep - 0.5) * 2.0;
    let metallic = mix(vec3<f32>(0.15), vec3<f32>(1.0), 1.0 - ramp);
    let warm = mix(vec3<f32>(0.6, 0.65, 0.75), vec3<f32>(1.0, 0.95, 0.8), params.tint * 0.01);
    let chrome = metallic * warm;
    return vec4<f32>(clamp01(mix(c.rgb, chrome, c.a)), c.a);
}
