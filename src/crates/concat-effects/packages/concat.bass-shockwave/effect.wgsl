struct Params { strength: f32, speed: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let center = vec2<f32>(0.5, 0.5);
    let d = uv - center;
    let dist = length(d);
    let phase = fract(frame.time * params.speed);
    let shock_ring = abs(dist - phase * 0.7);
    let shock = smoothstep(0.08, 0.0, shock_ring) * (1.0 - phase) * (params.strength * 0.01);
    let warp_uv = uv + normalize(d) * shock * 0.05;
    let c = sample(warp_uv);
    let highlight = c.rgb + vec3<f32>(shock * 0.3);
    return vec4<f32>(clamp01(highlight), c.a);
}
