struct Params { density: f32, speed: f32 }

// Flakes carry a depth: nearer ones fall faster, sway wider, glow bigger and
// brighter, and read warmer-white; farther ones are small, dim, cool-tinted
// pinpricks - the parallax a real snowfall has, in place of one flat layer
// of uniform dots.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let k = params.density * 0.01;
    var glow = vec3<f32>(0.0);
    for (var i: i32 = 0; i < 20; i++) {
        let seed = f32(i) * 11.0;
        let depth = hash(vec2<f32>(seed, 4.0), 0.0);
        let fall = fract(frame.time * params.speed * (0.06 + depth * 0.22) + hash(vec2<f32>(seed, 0.0), 0.0));
        let sway = sin(fall * 6.283 * 1.5 + seed) * 0.04 * (0.4 + depth);
        let x = fract(hash(vec2<f32>(seed, 1.0), 0.0) + sway);
        let p = vec2<f32>(x, fall);
        let d = distance(uv, p);
        let radius = mix(0.0025, 0.012, depth);
        let twinkle = 0.82 + 0.18 * sin(frame.time * 8.0 + seed * 3.0);
        let falloff = exp(-(d * d) / (radius * radius)) * twinkle;
        let tint = mix(vec3<f32>(0.82, 0.9, 1.0), vec3<f32>(1.0), depth);
        glow += tint * falloff * mix(0.35, 1.0, depth);
    }
    return vec4<f32>(clamp01(c.rgb + glow * k), c.a);
}
