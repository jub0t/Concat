struct Params { hue: f32, speed: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let wave = 0.5 + 0.5 * sin(uv.x * 6.283 * 1.5 + frame.time * params.speed);
    let base = mix(vec3<f32>(0.2, 0.8, 1.0), vec3<f32>(1.0, 0.3, 0.8), wave);
    let grad = hue_rotate(base, params.hue);
    return vec4<f32>(clamp01(mix(c.rgb, grad, c.a)), c.a);
}
