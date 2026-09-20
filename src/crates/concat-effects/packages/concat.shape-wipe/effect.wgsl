struct Params { shape: f32, softness: f32 }

// A parametric heart curve: r(theta) = sin(theta)*sqrt(|cos theta|)/(sin theta + 1.4),
// the denominator never nearing zero.
fn heart_d(p: vec2<f32>) -> f32 {
    let x = p.x;
    let y = -p.y - 0.1;
    let r = length(vec2<f32>(x, y));
    let theta = atan2(x, y);
    let boundary = 0.55 * (sin(theta) * sqrt(abs(cos(theta))) / (sin(theta) + 1.4) + 1.1);
    return r - boundary;
}

// A five-point shape by snapping the radius toward the nearest of five
// symmetric angles.
fn star_d(p: vec2<f32>) -> f32 {
    let r = length(p);
    let a = atan2(p.y, p.x);
    let points = 5.0;
    let seg = 6.28318530718 / points;
    let m = cos(floor(a / seg + 0.5) * seg - a) * 0.35 + 0.65;
    return r - 0.75 * m;
}

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = smoothstep(0.0, 1.0, progress);
    let c = (uv - vec2<f32>(0.5)) * 2.0;
    var d: f32;
    if (params.shape < 0.5) {
        d = length(c);
    } else if (params.shape < 1.5) {
        d = heart_d(c);
    } else {
        d = star_d(c);
    }
    let threshold = p * 3.0 - 1.0;
    let soft = max(params.softness * 0.01, 0.02) * 1.5;
    let m = smoothstep(threshold - soft, threshold + soft, d);
    return mix(to_at(uv), from_at(uv), m);
}
