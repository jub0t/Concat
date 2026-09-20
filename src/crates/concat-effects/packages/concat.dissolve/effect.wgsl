// A straight cross-dissolve: the outgoing picture fades into the incoming one
// over the cut. The reference every other transition is measured against, and
// the fallback the CPU and a GPU-less export degrade to.
fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    return mix(from_at(uv), to_at(uv), progress);
}
