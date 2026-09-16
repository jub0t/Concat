struct Params { position: f32, feather: f32, invert: f32 }

// Keeps the picture left of a soft vertical edge.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let edge = params.position / 100.0;
    let soft = params.feather / 100.0 + 0.0005;
    let keep = abs(params.invert - clamp((edge + soft * 0.5 - uv.x) / soft, 0.0, 1.0));
    return vec4<f32>(c.rgb, c.a * keep);
}
