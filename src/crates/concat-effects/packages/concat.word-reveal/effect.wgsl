struct Params { style: f32, duration: f32, softness: f32 }

// `reveal_order(uv)` is baked with the title: 0 for its first word, 1 for
// its last, 0 everywhere off a title (a no-op there, since progress
// starting at 0 would otherwise hide everything). Progress comes from
// `clip_time`, not a knob: the reveal starts the instant the clip does
// and finishes `duration` seconds later, so dropping the effect on a
// title is the whole gesture - nothing to key by hand. A word crosses
// from hidden to shown as progress passes its own order, softened by
// `softness` into a band so words do not pop in lock-step.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    let order = reveal_order(uv);
    let p = clamp(frame.clip_time / max(params.duration, 0.05), 0.0, 1.0);
    let soft = max(params.softness / 100.0, 0.001);
    let shown = 1.0 - smoothstep(p, p + soft, order);

    if (params.style < 0.5) {
        // Fade: the word's own alpha ramps in.
        return vec4<f32>(c.rgb, c.a * shown);
    } else if (params.style < 1.5) {
        // Blur: the word comes into focus as it fades in.
        let hidden = 1.0 - shown;
        let blurred = soften(uv, hidden * 14.0);
        return vec4<f32>(mix(c.rgb, blurred, hidden), c.a * shown);
    } else {
        // Typewriter: a hard cut, no softness, one word at a time.
        let cut = 1.0 - step(p, order);
        return vec4<f32>(c.rgb, c.a * cut);
    }
}
