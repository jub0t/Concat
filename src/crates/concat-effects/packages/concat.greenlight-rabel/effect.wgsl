struct Params { amount: f32, speed: f32, leaks: f32 }

// Lengths of the strip and its holes are in units of the layer's short
// side, so the film keeps its proportions at 16:9, 9:16 and 1:1 alike.

fn smooth_noise(x: f32, seed: f32) -> f32 {
    let i = floor(x);
    let f = fract(x);
    let a = hash(vec2<f32>(i, 0.0), seed);
    let b = hash(vec2<f32>(i + 1.0, 0.0), seed);
    return mix(a, b, f * f * (3.0 - 2.0 * f));
}

fn rounded_box(p: vec2<f32>, half_size: vec2<f32>, r: f32) -> f32 {
    let d = abs(p) - half_size + vec2<f32>(r);
    return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0) - r;
}

// One burst of light for each beat: a colour picked from green, gold and
// blue, pouring in from a hashed point on the frame's edge, swelling and
// dying away within the beat.
fn burst(uv: vec2<f32>, dims: vec2<f32>, t: f32, seed: f32) -> vec3<f32> {
    let beat = floor(t);
    let local = t - beat;
    let h1 = hash(vec2<f32>(beat, seed), 1.0);
    let h2 = hash(vec2<f32>(beat, seed), 2.0);
    let h3 = hash(vec2<f32>(beat, seed), 3.0);
    var tint = vec3<f32>(0.25, 1.0, 0.35);
    if (h1 > 0.66) {
        tint = vec3<f32>(1.0, 0.68, 0.12);
    } else if (h1 > 0.33) {
        tint = vec3<f32>(0.3, 0.6, 1.0);
    }
    let along = h2 * 4.0;
    var origin = vec2<f32>(fract(along), 0.0);
    if (along >= 3.0) {
        origin = vec2<f32>(0.0, fract(along));
    } else if (along >= 2.0) {
        origin = vec2<f32>(1.0, fract(along));
    } else if (along >= 1.0) {
        origin = vec2<f32>(fract(along), 1.0);
    }
    let d = length((uv - origin) * dims);
    let reach = 0.35 + 0.45 * h3;
    let shape = exp(-d * d / (reach * reach));
    let swell = smoothstep(0.0, 0.25, local) * (1.0 - smoothstep(0.45, 1.0, local));
    return tint * shape * swell;
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let amount = params.amount / 100.0;
    let side = max(min(frame.size.x, frame.size.y), 1.0);
    let dims = frame.size / side;
    let t = frame.time * (0.3 + params.speed / 100.0 * 2.1);

    let shake = (vec2<f32>(smooth_noise(t * 9.0, 11.0), smooth_noise(t * 9.0, 12.0)) - vec2<f32>(0.5)) * 0.008 * amount;
    let p = (uv - vec2<f32>(0.5)) * dims;
    let r2 = dot(p, p) / dot(dims * 0.5, dims * 0.5);
    let bulged = vec2<f32>(0.5) + (uv - vec2<f32>(0.5)) * (1.0 - 0.08 * amount * (1.0 - r2));
    let src = clamp(bulged + shake, vec2<f32>(0.0), vec2<f32>(1.0));
    let c = sample(src);
    var out = c.rgb;

    out = tint_midtones(out, vec3<f32>(-0.02, 0.03, -0.01), amount);

    let leaks = params.leaks / 100.0;
    let light = burst(uv, dims, t * 0.55, 0.0) + burst(uv, dims, t * 0.55 + 0.5, 5.0) * 0.6;
    let flicker = 0.8 + 0.2 * smooth_noise(t * 14.0, 21.0);
    out = screen(out, min(light * leaks * 1.6 * flicker * amount, vec3<f32>(1.0)));

    out = vignette(out, uv, 0.55 * amount);

    let q = uv * dims;
    let band = 0.085;
    let from_edge = min(q.y, dims.y - q.y);
    let strip = 1.0 - smoothstep(band - 1.0 / side, band, from_edge);
    let pitch = 0.075;
    let x = q.x + t * 0.12;
    let cellx = (fract(x / pitch) - 0.5) * pitch;
    let mid = band * 0.5;
    let celly = from_edge - mid;
    let hole_d = rounded_box(vec2<f32>(cellx, celly), vec2<f32>(0.018, 0.022), 0.006);
    let hole = 1.0 - smoothstep(-0.5 / side, 0.5 / side, hole_d);
    let film = mix(vec3<f32>(0.03, 0.035, 0.03), vec3<f32>(0.92, 0.95, 0.9), hole);
    out = mix(out, film, strip * amount);

    out = out + grain_at(uv, 0.05 * amount);
    return vec4<f32>(out, c.a);
}
