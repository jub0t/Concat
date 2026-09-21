struct Params { speed: f32, direction: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let dir = mix(1.0, -1.0, step(0.5, params.direction));
    let scrolled = fract(uv.x + frame.time * params.speed * 0.1 * dir);
    return sample(vec2<f32>(scrolled, uv.y));
}
