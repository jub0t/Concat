struct Params { width: f32, height: f32, feather: f32, x: f32, y: f32, invert: f32 }

// A soft-edged rectangle keeps the picture; outside it, nothing.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let px = uv * frame.size;
    let short = min(frame.size.x, frame.size.y);
    let soft = params.feather / 200.0 * short + 0.5;
    let centre = vec2<f32>(params.x, params.y) / 100.0 * frame.size;
    let half = vec2<f32>(params.width / 200.0 * frame.size.x, params.height / 200.0 * frame.size.y);
    let ax = clamp((half.x + soft * 0.5 - abs(px.x - centre.x)) / soft, 0.0, 1.0);
    let ay = clamp((half.y + soft * 0.5 - abs(px.y - centre.y)) / soft, 0.0, 1.0);
    let keep = abs(params.invert - ax * ay);
    return vec4<f32>(c.rgb, c.a * keep);
}
