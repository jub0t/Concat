struct Params { strength: f32 }

// The bulge of a very wide lens: the middle of the picture magnified and
// the edge drawn in, each point pulled towards the middle by a curve that
// keeps the corners where they were and never folds back on itself, so the
// whole frame stays filled. At full strength the middle is two and a half
// times its size.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let s = params.strength / 100.0 * 0.6;
    let aspect = frame.size.x / frame.size.y;
    // Out from the middle, in units where a corner is 1.
    let p = (uv - vec2<f32>(0.5)) * vec2<f32>(aspect, 1.0);
    let corner = length(vec2<f32>(aspect, 1.0) * 0.5);
    let r = length(p) / corner;
    let pulled = (1.0 - s) + s * r * r;
    return sample(vec2<f32>(0.5) + p * pulled / vec2<f32>(aspect, 1.0));
}
