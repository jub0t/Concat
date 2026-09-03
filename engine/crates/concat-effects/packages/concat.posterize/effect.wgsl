struct Params { levels: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let n = max(round(params.levels), 2.0);
    return vec4<f32>(floor(c.rgb * n) / (n - 1.0) * ((n - 1.0) / n) + vec3<f32>(0.0), c.a);
}
