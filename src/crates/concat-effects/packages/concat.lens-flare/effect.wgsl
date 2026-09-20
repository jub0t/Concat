struct Params { posx: f32, posy: f32, amount: f32 }

// The hotspot gets a four-point sparkle star, and the echoes drift from
// warm to cool with distance and each carry a thin ring - the chromatic,
// glinting look a real lens's glass and aperture blades add, not just a
// row of plain dots fading down a line.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let pos = vec2<f32>(params.posx, params.posy) * 0.01;
    let d = distance(uv, pos);
    let glow = exp(-d * d * 40.0) * 1.2;
    let a = atan2(uv.y - pos.y, uv.x - pos.x);
    let spikes = pow(abs(cos(a * 2.0)), 12.0) * exp(-d * 6.0);
    var tint = vec3<f32>(1.0, 0.9, 0.75) * (glow + spikes * 0.6);
    let dir = vec2<f32>(0.5) - pos;
    for (var i: i32 = 1; i <= 4; i++) {
        let t = f32(i) * 0.28;
        let p = pos + dir * (1.0 + t);
        let dd = distance(uv, p);
        let echo = exp(-dd * dd * 300.0) * 0.35 / f32(i);
        let ring = exp(-abs(dd - 0.02) * 80.0) * 0.12 / f32(i);
        let hue = mix(vec3<f32>(1.0, 0.9, 0.75), vec3<f32>(0.6, 0.8, 1.0), f32(i) / 4.0);
        tint += hue * (echo + ring);
    }
    let k = params.amount * 0.01;
    return vec4<f32>(clamp01(c.rgb + tint * k), c.a);
}
