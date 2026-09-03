struct Params {
    exposure: f32,
    brightness: f32,
    contrast: f32,
    saturation: f32,
    temperature: f32,
    tint: f32,
    shadows: f32,
    highlights: f32,
    sharpen: f32,
    vignette: f32,
    fade: f32,
}

fn kelvin(k: f32) -> vec3<f32> {
    let t = clamp(k, 1000.0, 40000.0) / 100.0;
    var r: f32;
    var g: f32;
    var b: f32;
    if (t <= 66.0) {
        r = 1.0;
        g = clamp((99.4708 * log(t) - 161.1196) / 255.0, 0.0, 1.0);
        if (t <= 19.0) {
            b = 0.0;
        } else {
            b = clamp((138.5177 * log(t - 10.0) - 305.0448) / 255.0, 0.0, 1.0);
        }
    } else {
        r = clamp(329.6987 * pow(t - 60.0, -0.1332) / 255.0, 0.0, 1.0);
        g = clamp(288.1222 * pow(t - 60.0, -0.0755) / 255.0, 0.0, 1.0);
        b = 1.0;
    }
    return vec3<f32>(r, g, b);
}

// The manual colour panel, in the order a colourist works: exposure and
// balance, then tone, then the edge and the frame.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let t = texel();
    let src = sample(uv);
    var c = src.rgb;

    // Exposure in stops, then brightness as an offset.
    c = c * exp2(params.exposure) + vec3<f32>(params.brightness / 100.0 * 0.5);

    // White balance about daylight, and green to magenta.
    c = c * (kelvin(params.temperature) / kelvin(6500.0));
    c.g = c.g - params.tint / 100.0 * 0.25;

    // Contrast about middle grey, saturation about luminance.
    c = (c - vec3<f32>(0.5)) * (1.0 + params.contrast / 100.0) + vec3<f32>(0.5);
    c = mix(vec3<f32>(luma(c)), c, 1.0 + params.saturation / 100.0 * 2.0);

    // Shadows and highlights lift or crush their end; fade lifts the black.
    let l = luma(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)));
    let low = (1.0 - smoothstep(0.0, 0.5, l)) * params.shadows / 100.0 * 0.18;
    let high = smoothstep(0.5, 1.0, l) * params.highlights / 100.0 * 0.18;
    c = c + vec3<f32>(low + high);
    let lift = params.fade / 100.0 * 0.25;
    c = c * (1.0 - lift) + vec3<f32>(lift);

    // Sharpen: the difference from a small blur, scaled.
    if (params.sharpen > 0.0) {
        var blur = vec3<f32>(0.0);
        for (var y: i32 = -1; y <= 1; y++) {
            for (var x: i32 = -1; x <= 1; x++) {
                blur += sample(uv + vec2<f32>(f32(x), f32(y)) * t).rgb;
            }
        }
        blur /= 9.0;
        c = c + (src.rgb - blur) * params.sharpen / 100.0 * 2.0;
    }

    // Vignette: darker towards the corners.
    if (params.vignette > 0.0) {
        let d = length((uv - vec2<f32>(0.5)) * vec2<f32>(2.0));
        let v = params.vignette / 100.0;
        let fall = smoothstep(1.4 - v * 0.9, 1.6 - v * 0.3, d);
        c = c * (1.0 - fall * (0.4 + 0.6 * v));
    }

    return vec4<f32>(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), src.a);
}
