struct Params { strength: f32, speed: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let t = frame.time * params.speed;
    let beam1 = abs(uv.y - 0.5 - 0.35 * sin(t + uv.x * 2.0));
    let beam2 = abs(uv.y - 0.5 + 0.35 * cos(t * 1.3 - uv.x * 3.0));
    let laser1 = exp(-beam1 * 80.0) * vec3<f32>(0.1, 1.0, 0.4);
    let laser2 = exp(-beam2 * 80.0) * vec3<f32>(1.0, 0.1, 0.6);
    let lasers = (laser1 + laser2) * (params.strength * 0.01);
    return vec4<f32>(clamp01(c.rgb + lasers), c.a);
}
