struct Params { angle: f32, radius: f32 }

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let center = vec2<f32>(0.5, 0.5);
    let d = uv - center;
    let r = length(d);
    let max_r = params.radius * 0.01;
    if (r >= max_r || r == 0.0) {
        return sample(uv);
    }
    let factor = (1.0 - r / max_r);
    let theta = factor * factor * (params.angle * 0.017453);
    let cos_t = cos(theta);
    let sin_t = sin(theta);
    let rotated = vec2<f32>(
        d.x * cos_t - d.y * sin_t,
        d.x * sin_t + d.y * cos_t
    );
    return sample(center + rotated);
}
