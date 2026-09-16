struct Params { dust: f32, leak: f32 }

// One mote: a soft dot drifting down the frame on its own lane, its lane,
// size, speed and sway all hashed from its number, so no two agree and
// the same frame always draws the same dust.
fn mote(uv: vec2<f32>, k: f32) -> f32 {
    let a = hash(vec2<f32>(k, 1.0), 3.0);
    let b = hash(vec2<f32>(k, 2.0), 5.0);
    let c = hash(vec2<f32>(k, 3.0), 7.0);
    let speed = 0.02 + b * 0.05;
    let y = fract(a - frame.time * speed);
    let x = fract(c + sin(frame.time * (0.3 + a) + k) * 0.02);
    let aspect = frame.size.x / max(frame.size.y, 1.0);
    let d = vec2<f32>((uv.x - x) * aspect, uv.y - y);
    let radius = 0.003 + b * 0.006;
    return (1.0 - smoothstep(0.0, radius, length(d))) * (0.4 + c * 0.6);
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let leak = params.leak / 100.0;
    let dust = params.dust / 100.0;
    var out = matte(c.rgb, 0.04, 0.97);
    out = split_tone(out, vec3<f32>(0.0), vec3<f32>(0.12, 0.03, -0.08), leak);
    let wash = (1.0 - smoothstep(0.0, 0.9, distance(uv, vec2<f32>(1.05, -0.1)))) * leak * 0.55;
    out = vec3<f32>(1.0) - (vec3<f32>(1.0) - out) * (vec3<f32>(1.0) - vec3<f32>(1.0, 0.72, 0.4) * wash);
    var glow = 0.0;
    for (var k: i32 = 0; k < 28; k++) {
        glow += mote(uv, f32(k));
    }
    out = out + vec3<f32>(1.0, 0.95, 0.85) * min(glow, 1.0) * dust * 0.8;
    return vec4<f32>(clamp01(out), c.a);
}
