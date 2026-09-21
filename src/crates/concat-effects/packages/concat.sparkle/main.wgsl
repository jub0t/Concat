struct Params { amount: f32, threshold: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let thresh = params.threshold * 0.01;
    let lum = luma(c.rgb);
    let star_intensity = smoothstep(thresh, 1.0, lum);

    let cell_scale = vec2<f32>(32.0, 18.0);
    let cell_coord = uv * cell_scale;
    let cell_id = floor(cell_coord);
    let cell_uv = fract(cell_coord) - 0.5;

    let center_uv = (cell_id + 0.5) / cell_scale;
    let center_lum = luma(sample(center_uv).rgb);
    let cell_star = smoothstep(thresh, 1.0, center_lum);

    let t = frame.time * 2.0 + hash(cell_id, 1.0) * 6.28;
    let cos_t = cos(t);
    let sin_t = sin(t);
    let rot_p = vec2<f32>(cell_uv.x * cos_t - cell_uv.y * sin_t, cell_uv.x * sin_t + cell_uv.y * cos_t);
    let ray1 = 1.0 / (abs(rot_p.x) * 40.0 + abs(rot_p.y) * 4.0 + 0.1);
    let ray2 = 1.0 / (abs(rot_p.y) * 40.0 + abs(rot_p.x) * 4.0 + 0.1);
    let star = max(ray1, ray2) * cell_star * 0.15;

    let shimmer = 0.7 + 0.3 * sin(frame.time * 4.0 + cell_id.x * 3.0 + cell_id.y * 7.0);
    let glint = (star_intensity * 0.2 + star) * shimmer * (params.amount * 0.02);
    let out_rgb = clamp01(c.rgb + vec3<f32>(glint));
    return vec4<f32>(out_rgb, c.a);
}
