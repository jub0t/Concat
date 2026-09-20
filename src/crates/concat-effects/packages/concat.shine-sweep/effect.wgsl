struct Params { speed: f32, width: f32, angle: f32 }

// A soft band travels along `angle` on a continuous loop; wherever the
// layer is transparent it stays transparent, so the sweep only ever shows
// through actual ink.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let a = radians(params.angle);
    let axis = uv.x * cos(a) + uv.y * sin(a);
    let pos = fract(frame.time * params.speed * 0.2);
    let d = abs(fract(axis - pos + 0.5) - 0.5);
    let band = smoothstep(params.width * 0.02, 0.0, d);
    let shine = band * c.a;
    return vec4<f32>(clamp01(c.rgb + vec3<f32>(shine)), c.a);
}
