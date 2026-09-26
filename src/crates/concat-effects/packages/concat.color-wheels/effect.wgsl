struct Params { lift: vec3<f32>, gamma: vec3<f32>, gain: vec3<f32> }

// Lift, gamma and gain on the display level (see `grade_wheels`): each
// wheel's puck a hue to push towards - red at the top - and its master a
// brighter or darker; lift moves black and leaves white, gamma bends the
// midtones, gain scales every level, past white too. What the Adjust
// panel's wheels do, alone.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    return vec4<f32>(grade_wheels(c.rgb, params.lift, params.gamma, params.gain), c.a);
}
