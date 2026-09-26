struct Params { stops: f32 }

// The light scaled by two to the stops: what opening or closing the lens
// does. Nothing is clipped, so a highlight pushed past white and brought
// back down by a later effect is the highlight it was.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    return vec4<f32>(exposure(c.rgb, params.stops), c.a);
}
