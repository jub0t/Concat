struct Params { colour: f32, punch: f32 }

// An S-curve, vibrance that spares skin, and a little edge.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let punch = params.punch / 100.0;
    var out = s_curve(c.rgb, punch * 0.8);
    out = vibrance(out, params.colour / 100.0 * 1.5);
    // Unsharp: the picture minus its blur, added back.
    let t = texel();
    let blur = (sample(uv + vec2<f32>(t.x, 0.0)).rgb + sample(uv - vec2<f32>(t.x, 0.0)).rgb
        + sample(uv + vec2<f32>(0.0, t.y)).rgb + sample(uv - vec2<f32>(0.0, t.y)).rgb) * 0.25;
    out = out + (c.rgb - blur) * punch * 0.6;
    return vec4<f32>(clamp01(out), c.a);
}
