struct Params { curvature: f32, scanlines: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let centered = (uv - 0.5) * 2.0;
    let r2 = dot(centered, centered);
    let warp = centered * (1.0 + (params.curvature * 0.002) * r2);
    let crt_uv = warp * 0.5 + 0.5;
    if (crt_uv.x < 0.0 || crt_uv.x > 1.0 || crt_uv.y < 0.0 || crt_uv.y > 1.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let c = sample(crt_uv);
    let scanline = sin(crt_uv.y * frame.size.y * 1.5) * 0.5 + 0.5;
    let dark = 1.0 - (params.scanlines * 0.01) * (1.0 - scanline) * 0.4;
    return vec4<f32>(c.rgb * dark, c.a);
}
