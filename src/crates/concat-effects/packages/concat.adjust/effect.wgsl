struct Params {
    exposure: f32,
    brightness: f32,
    contrast: f32,
    saturation: f32,
    temperature: f32,
    tint: f32,
    shadows: f32,
    highlights: f32,
    whites: f32,
    blacks: f32,
    sharpen: f32,
    vignette: f32,
    fade: f32,
    lift: vec3<f32>,
    gamma: vec3<f32>,
    gain: vec3<f32>,
    luma: array<vec4<f32>, 8>,
    red: array<vec4<f32>, 8>,
    green: array<vec4<f32>, 8>,
    blue: array<vec4<f32>, 8>,
}

// The manual colour panel, in the order a colourist works: exposure and
// balance, then tone, then the edge and the frame. What a lens or a lamp
// does is done in light - exposure in stops, the white balance, the
// vignette - and contrast in stops about middle grey; what an eye judges
// on the picture - brightness, saturation, the four tonal bands, the fade,
// the wheels and curves, sharpness - in the display encoding its sliders
// were drawn in, which here is never clipped: a highlight past white is
// carried on past it. Every knob at its default is the picture as it came.

/// `by` added to a display-encoded colour, as far down as black and no
/// further: a channel that is lit is not taken below nothing.
fn lifted(encoded: vec3<f32>, by: f32) -> vec3<f32> {
    let moved = encoded + vec3<f32>(by);
    return select(moved, max(moved, vec3<f32>(0.0)), encoded >= vec3<f32>(0.0));
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let t = texel();
    let src = sample(uv);

    // Exposure in stops, then brightness as a lift of the display level.
    var c = exposure(src.rgb, params.exposure);
    c = from_display(lifted(to_display(c), params.brightness / 100.0 * 0.5));

    // White balance about daylight, and green to magenta.
    c = white_balance(c, params.temperature, params.tint);

    // Contrast in stops about middle grey.
    c = contrast(c, 1.0 + params.contrast / 100.0);

    // Saturation about luminance, by the display level: a push richer by
    // up to three times, a pull greyer to none at all.
    var d = to_display(c);
    let s = params.saturation / 100.0;
    d = saturation(d, select((1.0 + s) * (1.0 + s), 1.0 + 2.0 * s, s > 0.0));

    // Shadows and highlights lift or crush their half of the range, blacks
    // and whites the extreme quarter of each, by the display level; fade
    // lifts the black and carries a level past white on past it.
    let l = luma(clamp(d, vec3<f32>(0.0), vec3<f32>(1.0)));
    let low = (1.0 - smoothstep(0.0, 0.5, l)) * params.shadows / 100.0 * 0.18;
    let high = smoothstep(0.5, 1.0, l) * params.highlights / 100.0 * 0.18;
    let black = (1.0 - smoothstep(0.0, 0.25, l)) * params.blacks / 100.0 * 0.1;
    let white = smoothstep(0.75, 1.0, l) * params.whites / 100.0 * 0.1;
    d = lifted(d, low + high + black + white);
    let matte = params.fade / 100.0 * 0.25;
    let below = min(d, vec3<f32>(1.0));
    d = below * (1.0 - matte) + vec3<f32>(matte) + (d - below);

    // The wheels, then the curves, on the display level too.
    d = grade_wheels(d, params.lift, params.gamma, params.gain);
    d = grade_curves(d, params.luma, params.red, params.green, params.blue);

    // Sharpen: the picture's difference from a small blur of it, scaled.
    if (params.sharpen > 0.0) {
        var blur = vec3<f32>(0.0);
        for (var y: i32 = -1; y <= 1; y++) {
            for (var x: i32 = -1; x <= 1; x++) {
                blur += to_display(sample(uv + vec2<f32>(f32(x), f32(y)) * t).rgb);
            }
        }
        blur /= 9.0;
        d = d + (to_display(src.rgb) - blur) * params.sharpen / 100.0 * 2.0;
    }
    c = from_display(d);

    // Vignette: the light falling off towards the corners, as a lens's does.
    if (params.vignette > 0.0) {
        let r = length((uv - vec2<f32>(0.5)) * vec2<f32>(2.0));
        let v = params.vignette / 100.0;
        let fall = smoothstep(1.4 - v * 0.9, 1.6 - v * 0.3, r);
        c = c * pow(1.0 - fall * (0.4 + 0.6 * v), 2.4);
    }

    return vec4<f32>(c, src.a);
}
