// A cheap fake of a rotating cube: each half of the cut squeezes its
// picture horizontally toward the shared edge and darkens it, the way a
// cube's face would foreshorten and fall into shadow as it turns away.
// The shade was drawn on the display's scale; `shade` is that darkening
// as a share of the light.
fn shade(display: f32) -> f32 {
    return pow(display, 2.4);
}

fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let gap = vec4<f32>(from_display(vec3<f32>(0.03)), 1.0);
    let p = smoothstep(0.0, 1.0, progress);
    if (p < 0.5) {
        let t = p * 2.0;
        let squeeze = cos(t * 1.5707963);
        let cx = (uv.x - 0.5) / max(squeeze, 0.001) + 0.5;
        if (cx < 0.0 || cx > 1.0) {
            return gap;
        }
        let c = from_at(vec2<f32>(cx, uv.y));
        return vec4<f32>(c.rgb * shade(mix(1.0, 0.35, t)), c.a);
    }
    let t = (p - 0.5) * 2.0;
    let squeeze = sin(t * 1.5707963);
    let cx = (uv.x - 0.5) / max(squeeze, 0.001) + 0.5;
    if (cx < 0.0 || cx > 1.0) {
        return gap;
    }
    let c = to_at(vec2<f32>(cx, uv.y));
    return vec4<f32>(c.rgb * shade(mix(0.35, 1.0, t)), c.a);
}
