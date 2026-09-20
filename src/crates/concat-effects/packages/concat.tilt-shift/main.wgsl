struct Params { blur: f32, position: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let center_y = params.position * 0.01;
    let dist = abs(uv.y - center_y);
    let blur_factor = smoothstep(0.1, 0.4, dist) * (params.blur * 0.01);
    var col = vec4<f32>(0.0);
    let rad = blur_factor * 8.0 * texel();
    col += sample(uv) * 0.36;
    col += sample(uv + vec2<f32>(0.0, rad.y * 1.5)) * 0.16;
    col += sample(uv - vec2<f32>(0.0, rad.y * 1.5)) * 0.16;
    col += sample(uv + vec2<f32>(rad.x * 1.5, 0.0)) * 0.16;
    col += sample(uv - vec2<f32>(rad.x * 1.5, 0.0)) * 0.16;
    let sat_col = saturation(col.rgb, 1.25);
    return vec4<f32>(mix(col.rgb, sat_col, 0.5), col.a);
}
