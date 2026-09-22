struct Params { grain: f32, weave: f32 }

fn draw_digit(p: vec2<f32>, d: i32) -> f32 {
    let ix = floor(p.x);
    let iy = floor(p.y);
    if (ix < 0.0 || ix > 2.0 || iy < 0.0 || iy > 4.0) { return 0.0; }
    if (d == 1) { return f32(ix == 2.0); }
    if (d == 2) { return f32(iy == 0.0 || iy == 2.0 || iy == 4.0 || (ix == 2.0 && iy <= 2.0) || (ix == 0.0 && iy >= 2.0)); }
    if (d == 3) { return f32(iy == 0.0 || iy == 2.0 || iy == 4.0 || ix == 2.0); }
    return 0.0;
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let jitter_x = (hash(vec2<f32>(floor(frame.time * 24.0), 0.0), 1.0) - 0.5) * params.weave * texel().x;
    let jitter_y = (hash(vec2<f32>(floor(frame.time * 24.0), 1.0), 2.0) - 0.5) * params.weave * texel().y;
    let warped_uv = uv + vec2<f32>(jitter_x, jitter_y);
    let c = sample(warped_uv);
    let n = hash(uv * 100.0, frame.time) - 0.5;
    let grain_val = n * (params.grain * 0.01) * 0.5;
    var col = mix(c.rgb, vec3<f32>(luma(c.rgb) * 1.1, luma(c.rgb) * 0.95, luma(c.rgb) * 0.75), 0.3);

    // 3-2-1 Countdown leader
    let center = vec2<f32>(0.5, 0.5);
    let d = uv - center;
    let r = length(d);
    let outer_ring = smoothstep(0.003, 0.0, abs(r - 0.28));
    let inner_ring = smoothstep(0.003, 0.0, abs(r - 0.22));
    let cross_x = smoothstep(0.002, 0.0, abs(uv.x - 0.5)) * step(r, 0.3);
    let cross_y = smoothstep(0.002, 0.0, abs(uv.y - 0.5)) * step(r, 0.3);
    let angle = (atan2(d.y, d.x) + 3.14159) / 6.28318;
    var leader = 0.0;
    if (frame.clip_time < 3.0) {
        let sweep = fract(frame.clip_time);
        let arm = smoothstep(0.01, 0.0, abs(angle - sweep)) * step(r, 0.28);
        let reticle = max(max(outer_ring, inner_ring), max(cross_x, cross_y)) + arm;
        let sec = i32(frame.clip_time);
        let num_p = (uv - vec2<f32>(0.485, 0.46)) / vec2<f32>(0.01, 0.016);
        let num = draw_digit(num_p, 3 - sec);
        leader = clamp(reticle + num, 0.0, 1.0);
    }
    col = mix(col, vec3<f32>(0.92, 0.88, 0.75), leader * 0.75);

    return vec4<f32>(clamp01(col + vec3<f32>(grain_val)), c.a);
}
