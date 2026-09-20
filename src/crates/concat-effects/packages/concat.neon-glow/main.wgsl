struct Params { glow: f32, speed: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let t = texel() * 2.0;
    let c_r = sample(uv + vec2<f32>(t.x, 0.0));
    let c_u = sample(uv + vec2<f32>(0.0, t.y));
    let edge = length(c.rgb - c_r.rgb) + length(c.rgb - c_u.rgb);
    let hue = fract(frame.time * params.speed * 0.1);
    let neon_col = vec3<f32>(
        0.5 + 0.5 * sin(hue * 6.283),
        0.5 + 0.5 * sin(hue * 6.283 + 2.094),
        0.5 + 0.5 * sin(hue * 6.283 + 4.188)
    );
    let glow_val = edge * (params.glow * 0.03) * neon_col;
    return vec4<f32>(clamp01(c.rgb + glow_val), c.a);
}
