struct Params { amount: f32 }

// Warm motes drifting in lazy circles, the way dust turns in a sunbeam -
// in place of a per-pixel noise threshold that just flickered on and off
// with the frame.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let k = params.amount * 0.01;
    var glow = vec3<f32>(0.0);
    for (var i: i32 = 0; i < 14; i++) {
        let seed = f32(i) * 17.0;
        let depth = hash(vec2<f32>(seed, 5.0), 0.0);
        let base = vec2<f32>(hash(vec2<f32>(seed, 1.0), 0.0), hash(vec2<f32>(seed, 2.0), 0.0));
        let t = frame.time * (0.15 + depth * 0.2) + seed;
        let drift = vec2<f32>(sin(t), cos(t * 0.8)) * mix(0.015, 0.05, depth);
        let p = fract(base + drift);
        let d = distance(uv, p);
        let radius = mix(0.0015, 0.005, depth);
        let sparkle = 0.7 + 0.3 * sin(frame.time * 6.0 + seed * 4.0);
        let falloff = exp(-(d * d) / (radius * radius)) * sparkle;
        glow += vec3<f32>(1.0, 0.92, 0.75) * falloff * mix(0.25, 0.9, depth);
    }
    return vec4<f32>(clamp01(c.rgb + glow * k), c.a);
}
