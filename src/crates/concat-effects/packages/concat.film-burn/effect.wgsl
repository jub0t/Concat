struct Params { heat: f32 }

// A warm flash triangular in time, peaking at the middle of the cut.
fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let base = mix(from_at(uv), to_at(uv), progress);
    let burn = smoothstep(0.0, 0.5, progress) * (1.0 - smoothstep(0.5, 1.0, progress)) * 4.0;
    let warm = vec3<f32>(1.0, 0.85, 0.6) * burn * (params.heat * 0.01);
    return vec4<f32>(clamp01(base.rgb + warm), base.a);
}
