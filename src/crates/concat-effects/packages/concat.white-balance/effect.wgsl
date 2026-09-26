struct Params { temperature: f32, tint: f32 }

// The picture as if lit at `temperature` kelvin rather than the daylight it
// was shot in - lower warmer, 6500 as shot - and turned towards magenta by
// `tint`, minus towards green: the eye's own adaptation, in light (see
// `white_balance`), a grey keeping its brightness. What the Adjust panel's
// temperature and tint do, alone.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    return vec4<f32>(white_balance(c.rgb, params.temperature, params.tint), c.a);
}
