struct Params { amount: f32 }

// Smooth value noise: a hashed grid of corners, bilinearly blended - the
// cheapest way to get soft, wispy variation instead of a single flat value.
fn noise2(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let n00 = hash(i, 0.0);
    let n10 = hash(i + vec2<f32>(1.0, 0.0), 0.0);
    let n01 = hash(i + vec2<f32>(0.0, 1.0), 0.0);
    let n11 = hash(i + vec2<f32>(1.0, 1.0), 0.0);
    let u = f * f * (vec2<f32>(3.0) - 2.0 * f);
    return mix(mix(n00, n10, u.x), mix(n01, n11, u.x), u.y);
}

// Two drifting octaves of noise, banked thicker toward the bottom of the
// frame - a ground fog with texture and a slow roll, not a flat wash.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let k = params.amount * 0.01;
    let drift = vec2<f32>(frame.time * 0.02, frame.time * 0.008);
    let n1 = noise2(uv * vec2<f32>(3.0, 2.0) + drift);
    let n2 = noise2(uv * vec2<f32>(6.5, 4.5) - drift * 1.6);
    let texture = n1 * 0.65 + n2 * 0.35;
    let ground = smoothstep(0.05, 0.95, uv.y);
    let density = clamp((texture * 0.6 + 0.4) * mix(0.3, 1.0, ground) * k * 1.6, 0.0, 1.0);
    let haze = fade(c.rgb, density * 0.55);
    return vec4<f32>(clamp01(saturation(haze, 1.0 - density * 0.6)), c.a);
}
