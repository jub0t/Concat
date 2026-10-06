struct Params { amount: f32, speed: f32, grid: f32 }

// One run, three seconds at the default speed, timed from the layer's or
// clip's own start. The cells arrive one after another in reading order,
// the first ninety hundredths of a second shared between them, each on
// its own clock from its arrival `o`:
//   o .. o+0.1    fades in, in colour, the picture at 150%
//   o+0.9 .. 1.1  flashes to white and comes back black and white, at 200%
//   o+1.8 .. 2.0  flashes again and comes back in colour, at 100%
// and before it arrives the cell is black.

fn ramp(t: f32, a: f32, b: f32) -> f32 {
    return smoothstep(a, b, t);
}

// Stops of light at `t` on a cell's clock: a stop on arrival dying away,
// then the two flashes, each ten stops at its peak.
fn stops(t: f32) -> f32 {
    let arrive = 1.0 - ramp(t, 0.1, 0.2);
    let first = ramp(t, 0.9, 1.0) * (1.0 - ramp(t, 1.0, 1.1));
    let second = ramp(t, 1.8, 1.9) * (1.0 - ramp(t, 1.9, 2.0));
    return arrive + 10.0 * (first + second);
}

fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let amount = params.amount / 100.0;
    let n = max(round(params.grid), 1.0);
    let cell = floor(clamp(uv, vec2<f32>(0.0), vec2<f32>(0.9999)) * n);
    let index = cell.y * n + cell.x;
    let pace = 0.5 + params.speed / 100.0 * 2.5;
    let t = frame.clip_time * pace - index * 0.9 / (n * n);

    let shown = ramp(t, 0.0, 0.1);
    let mono_mix = ramp(t, 0.967, 1.0) * (1.0 - ramp(t, 1.867, 1.9));
    let scale = mix(mix(1.5, 2.0, ramp(t, 0.967, 1.0)), 1.0, ramp(t, 1.867, 1.9));
    let at = vec2<f32>(0.5) + (uv - vec2<f32>(0.5)) / scale;
    var rgb = sample(at).rgb;

    let grey = smoothstep(0.05, 0.95, luma(rgb));
    rgb = mix(rgb, vec3<f32>(grey), mono_mix);
    let lit = from_display(rgb) * exp2(stops(t));
    rgb = to_display(min(lit, max(from_display(rgb), vec3<f32>(1.0))));

    let tile = rgb * shown;
    return vec4<f32>(mix(c.rgb, tile, amount), c.a);
}
