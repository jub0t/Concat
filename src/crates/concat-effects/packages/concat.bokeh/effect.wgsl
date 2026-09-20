struct Params { amount: f32 }

// Each circle is a bright core inside a soft halo rather than one
// smoothstep ring, drifts very slightly rather than sitting dead still, and
// leans warm or cool at random - the creamy, faintly alive look a real
// out-of-focus highlight has, not a flat white disc.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    var glow = vec3<f32>(0.0);
    for (var i: i32 = 0; i < 12; i++) {
        let seed = f32(i) * 9.0;
        let depth = hash(vec2<f32>(seed, 6.0), 0.0);
        let drift = vec2<f32>(sin(frame.time * 0.1 + seed), cos(frame.time * 0.08 + seed)) * 0.01;
        let p = vec2<f32>(hash(vec2<f32>(seed, 1.0), 0.0), hash(vec2<f32>(seed, 2.0), 0.0)) + drift;
        let r = mix(0.025, 0.075, depth);
        let d = distance(uv, p);
        let core = exp(-(d * d) / (r * r * 0.08));
        let halo = exp(-(d * d) / (r * r));
        let warm = hash(vec2<f32>(seed, 7.0), 0.0) > 0.5;
        let tint = select(vec3<f32>(0.55, 0.75, 1.0), vec3<f32>(1.0, 0.82, 0.55), warm);
        glow += tint * (core * 0.9 + halo * 0.35) * mix(0.3, 1.0, depth);
    }
    return vec4<f32>(clamp01(c.rgb + glow * params.amount * 0.01), c.a);
}
