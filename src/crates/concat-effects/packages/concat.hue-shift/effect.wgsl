struct Params { angle: f32 }

// A turn of the hue is a plain rotation in YIQ, where the two chroma axes
// are at right angles: convert, turn I and Q by the angle, convert back.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let a = radians(params.angle);
    let cs = cos(a);
    let sn = sin(a);
    let to_yiq = mat3x3<f32>(
        vec3<f32>(0.299, 0.596, 0.211),
        vec3<f32>(0.587, -0.274, -0.523),
        vec3<f32>(0.114, -0.322, 0.312),
    );
    let from_yiq = mat3x3<f32>(
        vec3<f32>(1.0, 1.0, 1.0),
        vec3<f32>(0.956, -0.272, -1.106),
        vec3<f32>(0.621, -0.647, 1.703),
    );
    let yiq = to_yiq * c.rgb;
    let turned = vec3<f32>(yiq.x, yiq.y * cs - yiq.z * sn, yiq.y * sn + yiq.z * cs);
    return vec4<f32>(clamp(from_yiq * turned, vec3<f32>(0.0), vec3<f32>(1.0)), c.a);
}
