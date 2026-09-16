struct Params { size: f32, feather: f32, x: f32, y: f32, invert: f32 }

// A soft-edged disc keeps the picture; outside it, nothing.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let px = uv * frame.size;
    let short = min(frame.size.x, frame.size.y);
    let radius = params.size / 200.0 * short;
    let soft = params.feather / 200.0 * short + 0.5;
    let centre = vec2<f32>(params.x, params.y) / 100.0 * frame.size;
    let d = distance(px, centre);
    var keep = clamp((radius + soft * 0.5 - d) / soft, 0.0, 1.0);
    keep = abs(params.invert - keep);
    return vec4<f32>(c.rgb, c.a * keep);
}
