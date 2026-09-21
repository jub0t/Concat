struct Params { blur: f32, spread: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let radius = max(params.blur, 0.1) * 2.0;
    let t = texel() * radius;
    var sum = vec4<f32>(0.0);
    for (var y: i32 = -1; y <= 1; y++) {
        for (var x: i32 = -1; x <= 1; x++) {
            sum += sample(uv + vec2<f32>(f32(x), f32(y)) * t);
        }
    }
    let blurred = sum / 9.0;
    let halo = clamp(blurred.a * params.spread * 0.01, 0.0, 1.0);
    let out_rgb = mix(blurred.rgb, c.rgb, c.a);
    let out_a = clamp(max(c.a, halo), 0.0, 1.0);
    return vec4<f32>(out_rgb, out_a);
}
