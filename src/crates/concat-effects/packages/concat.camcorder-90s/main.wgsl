struct Params { noise: f32, lines: f32 }

fn draw_rec(uv: vec2<f32>) -> f32 {
    let p = (uv - vec2<f32>(0.09, 0.068)) / vec2<f32>(0.004, 0.005);
    let ix = floor(p.x);
    let iy = floor(p.y);
    if (iy < 0.0 || iy > 4.0) { return 0.0; }
    if (ix >= 0.0 && ix <= 2.0) {
        if (ix == 0.0 || iy == 0.0 || iy == 2.0 || (ix == 2.0 && (iy == 1.0 || iy >= 3.0))) {
            return 1.0;
        }
    }
    if (ix >= 4.0 && ix <= 6.0) {
        let x = ix - 4.0;
        if (x == 0.0 || iy == 0.0 || iy == 2.0 || iy == 4.0) {
            return 1.0;
        }
    }
    if (ix >= 8.0 && ix <= 10.0) {
        let x = ix - 8.0;
        if (x == 0.0 || iy == 0.0 || iy == 4.0) {
            return 1.0;
        }
    }
    return 0.0;
}

fn draw_battery(uv: vec2<f32>) -> f32 {
    if (uv.x >= 0.88 && uv.x <= 0.94 && uv.y >= 0.068 && uv.y <= 0.092) {
        if (uv.x < 0.883 || uv.x > 0.937 || uv.y < 0.071 || uv.y > 0.089) {
            return 1.0;
        }
        if (uv.x >= 0.888 && uv.x <= 0.908 && uv.y >= 0.074 && uv.y <= 0.086) { return 1.0; }
        if (uv.x >= 0.914 && uv.x <= 0.932 && uv.y >= 0.074 && uv.y <= 0.086) { return 1.0; }
    }
    if (uv.x > 0.94 && uv.x <= 0.945 && uv.y >= 0.075 && uv.y <= 0.085) {
        return 1.0;
    }
    return 0.0;
}

fn draw_digit(p: vec2<f32>, d: i32) -> f32 {
    let ix = floor(p.x);
    let iy = floor(p.y);
    if (ix < 0.0 || ix > 2.0 || iy < 0.0 || iy > 4.0) { return 0.0; }
    if (d == 0) { return f32(ix == 0.0 || ix == 2.0 || iy == 0.0 || iy == 4.0); }
    if (d == 1) { return f32(ix == 2.0); }
    if (d == 2) { return f32(iy == 0.0 || iy == 2.0 || iy == 4.0 || (ix == 2.0 && iy <= 2.0) || (ix == 0.0 && iy >= 2.0)); }
    if (d == 3) { return f32(iy == 0.0 || iy == 2.0 || iy == 4.0 || ix == 2.0); }
    if (d == 4) { return f32(ix == 2.0 || iy == 2.0 || (ix == 0.0 && iy <= 2.0)); }
    if (d == 5) { return f32(iy == 0.0 || iy == 2.0 || iy == 4.0 || (ix == 0.0 && iy <= 2.0) || (ix == 2.0 && iy >= 2.0)); }
    if (d == 6) { return f32(ix == 0.0 || iy == 0.0 || iy == 2.0 || iy == 4.0 || (ix == 2.0 && iy >= 2.0)); }
    if (d == 7) { return f32(iy == 0.0 || ix == 2.0); }
    if (d == 8) { return f32(ix == 0.0 || ix == 2.0 || iy == 0.0 || iy == 2.0 || iy == 4.0); }
    if (d == 9) { return f32(ix == 2.0 || iy == 0.0 || iy == 2.0 || iy == 4.0 || (ix == 0.0 && iy <= 2.0)); }
    return 0.0;
}

fn draw_date(uv: vec2<f32>) -> f32 {
    let p = (uv - vec2<f32>(0.06, 0.85)) / vec2<f32>(0.004, 0.005);
    let char_idx = floor(p.x / 4.0);
    let digit_p = vec2<f32>(fract(p.x / 4.0) * 4.0, p.y);
    if (char_idx == 0.0) { return draw_digit(digit_p, 1); }
    if (char_idx == 1.0) { return draw_digit(digit_p, 9); }
    if (char_idx == 2.0) { return draw_digit(digit_p, 9); }
    if (char_idx == 3.0) { return draw_digit(digit_p, 8); }
    return 0.0;
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let is_bottom = step(0.92, uv.y);
    let tracking_noise = (hash(vec2<f32>(uv.x, frame.time), 3.0) - 0.5) * is_bottom * (params.noise * 0.005);
    let sample_uv = uv + vec2<f32>(tracking_noise, 0.0);
    let c = sample(sample_uv);
    let scanline = sin(uv.y * 300.0 + frame.time * 10.0) * 0.5 + 0.5;
    let interlacing = 1.0 - (params.lines * 0.01) * (1.0 - scanline) * 0.2;
    var tape_color = saturation(c.rgb, 0.85) * interlacing;

    let rec_center = vec2<f32>(0.065, 0.078);
    let rec_dist = length(uv - rec_center);
    let rec_blink = step(0.4, fract(frame.time));
    if (rec_dist < 0.01 && rec_blink > 0.5) {
        tape_color = vec3<f32>(1.0, 0.15, 0.15);
    }
    let rec_text = draw_rec(uv);
    if (rec_text > 0.5) {
        tape_color = vec3<f32>(0.95, 0.95, 0.95);
    }

    let bat = draw_battery(uv);
    if (bat > 0.5) {
        tape_color = vec3<f32>(0.95, 0.95, 0.95);
    }

    let date_val = draw_date(uv);
    if (date_val > 0.5) {
        tape_color = vec3<f32>(0.95, 0.95, 0.95);
    }

    return vec4<f32>(tape_color, c.a);
}
