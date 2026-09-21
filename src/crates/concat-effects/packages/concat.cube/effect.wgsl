// A cheap fake of a rotating cube: each half of the cut squeezes its
// picture horizontally toward the shared edge and darkens it, the way a
// cube's face would foreshorten and fall into shadow as it turns away.
fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    let p = smoothstep(0.0, 1.0, progress);
    if (p < 0.5) {
        let t = p * 2.0;
        let squeeze = cos(t * 1.5707963);
        let cx = (uv.x - 0.5) / max(squeeze, 0.001) + 0.5;
        if (cx < 0.0 || cx > 1.0) {
            return vec4<f32>(0.03, 0.03, 0.03, 1.0);
        }
        let c = from_at(vec2<f32>(cx, uv.y));
        return vec4<f32>(c.rgb * mix(1.0, 0.35, t), c.a);
    }
    let t = (p - 0.5) * 2.0;
    let squeeze = sin(t * 1.5707963);
    let cx = (uv.x - 0.5) / max(squeeze, 0.001) + 0.5;
    if (cx < 0.0 || cx > 1.0) {
        return vec4<f32>(0.03, 0.03, 0.03, 1.0);
    }
    let c = to_at(vec2<f32>(cx, uv.y));
    return vec4<f32>(c.rgb * mix(0.35, 1.0, t), c.a);
}
