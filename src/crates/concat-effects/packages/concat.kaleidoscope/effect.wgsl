struct Params { segments: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = uv - vec2<f32>(0.5);
    let r = length(c);
    var a = atan2(c.y, c.x);
    let seg = 6.28318530718 / max(params.segments, 3.0);
    a = abs((a - seg * floor(a / seg)) - seg * 0.5);
    let p = vec2<f32>(cos(a), sin(a)) * r + vec2<f32>(0.5);
    return sample(p);
}
