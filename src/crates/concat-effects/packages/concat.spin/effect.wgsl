struct Params { turns: f32 }

fn in_bounds(uv: vec2<f32>) -> bool {
    return uv.x >= 0.0 && uv.x <= 1.0 && uv.y >= 0.0 && uv.y <= 1.0;
}

fn spin_uv(uv: vec2<f32>, angle: f32, scale: f32) -> vec2<f32> {
    let c = uv - vec2<f32>(0.5);
    let s = sin(angle);
    let co = cos(angle);
    let r = vec2<f32>(c.x * co - c.y * s, c.x * s + c.y * co) / max(scale, 0.0001);
    return r + vec2<f32>(0.5);
}

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = smoothstep(0.0, 1.0, progress);
    let full = params.turns * 6.28318530718;
    let ang = p * full;
    let a_uv = spin_uv(uv, ang, 1.0 - p * 0.9);
    let b_uv = spin_uv(uv, ang - full, 0.1 + p * 0.9);
    let a = select(vec4<f32>(0.0), from_at(a_uv), in_bounds(a_uv));
    let b = select(vec4<f32>(0.0), to_at(b_uv), in_bounds(b_uv));
    return mix(a, b, p);
}
