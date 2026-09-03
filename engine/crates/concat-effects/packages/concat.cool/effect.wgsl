struct Params { temperature: f32 }

// Colour temperature as a white balance shift: the picture as if it were
// lit at `temperature` kelvin while the camera was set for daylight.
fn kelvin(k: f32) -> vec3<f32> {
    let t = clamp(k, 1000.0, 40000.0) / 100.0;
    var r: f32;
    var g: f32;
    var b: f32;
    if (t <= 66.0) {
        r = 1.0;
        g = clamp((99.4708 * log(t) - 161.1196) / 255.0, 0.0, 1.0);
        if (t <= 19.0) {
            b = 0.0;
        } else {
            b = clamp((138.5177 * log(t - 10.0) - 305.0448) / 255.0, 0.0, 1.0);
        }
    } else {
        r = clamp(329.6987 * pow(t - 60.0, -0.1332) / 255.0, 0.0, 1.0);
        g = clamp(288.1222 * pow(t - 60.0, -0.0755) / 255.0, 0.0, 1.0);
        b = 1.0;
    }
    return vec3<f32>(r, g, b);
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let tint = kelvin(params.temperature) / kelvin(6500.0);
    return vec4<f32>(clamp(c.rgb * tint, vec3<f32>(0.0), vec3<f32>(1.0)), c.a);
}
