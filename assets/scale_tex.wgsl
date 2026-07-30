struct Vertex {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>
};

@vertex
fn vertex_main(@builtin(vertex_index) vertex_i: u32) -> Vertex {
    let vertices = array<Vertex, 3>(
        Vertex(vec4<f32>(-1.0, -3.0, 0.0, 1.0), vec2<f32>(0.0, 2.0)),
        Vertex(vec4<f32>( 3.0,  1.0, 0.0, 1.0), vec2<f32>(2.0, 0.0)),
        Vertex(vec4<f32>(-1.0,  1.0, 0.0, 1.0), vec2<f32>(0.0, 0.0))
    );

    return vertices[vertex_i];
}

@group(0) @binding(0)
var tex: texture_2d<f32>;
@group(0) @binding(1)
var smp: sampler;

@fragment
fn fragment_main(vertex: Vertex) -> @location(0) vec4<f32> {
    return textureSample(tex, smp, vertex.uv);
}
