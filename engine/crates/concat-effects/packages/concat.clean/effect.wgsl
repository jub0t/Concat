struct Params { amount: f32 }

// The quiet one: a gentle curve and a touch more colour.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let amount = params.amount / 100.0;
    var out = s_curve(c.rgb, amount * 0.35);
    out = saturation(out, 1.0 + amount * 0.12);
    return vec4<f32>(clamp01(out), c.a);
}
