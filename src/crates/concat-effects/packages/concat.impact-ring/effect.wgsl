struct Params { period: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let centre = vec2<f32>(0.5, 0.5);
    let d = distance(uv, centre);
    let t = fract(frame.time / max(params.period, 0.1));
    let radius = t * 0.9;
    let ring = exp(-pow((d - radius) * 30.0, 2.0)) * (1.0 - t);
    let glow = vec3<f32>(1.0, 0.92, 0.75) * ring;
    return vec4<f32>(clamp01(c.rgb + glow), clamp(c.a + ring * 0.8, 0.0, 1.0));
}
