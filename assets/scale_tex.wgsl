@vertex
fn vertex_main(@builtin(vertex_index) vertex_i: u32) -> @builtin(position) vec4<f32> {
    let pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>( 3.0,  1.0),
        vec2<f32>(-1.0,  1.0)
    );

    return vec4<f32>(pos[vertex_i], 0.0, 1.0);
}

@group(0) @binding(0)
var tex: texture_2d<f32>;
@group(0) @binding(1)
var smp: sampler;

@fragment
fn fragment_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let extent = vec2<f32>(textureDimensions(tex));
    let uv = pos.xy / extent;

    return textureSample(tex, smp, uv);
}
