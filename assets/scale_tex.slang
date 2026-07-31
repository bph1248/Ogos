struct Vertex {
    float4 pos: SV_Position;
    float2 uv: SV_Target0;
}

[shader("vertex")]
Vertex vertex_main(uint vertex_i: SV_VulkanVertexID) {
    Vertex vertices[3] = {
        { float4(-1.0, -3.0, 0.0, 1.0), float2(0.0, 2.0) },
        { float4( 3.0,  1.0, 0.0, 1.0), float2(2.0, 0.0) },
        { float4(-1.0,  1.0, 0.0, 1.0), float2(0.0, 0.0) }
    };

    return vertices[vertex_i];
}

[[vk::binding(0, 0)]]
Texture2D tex;
[[vk::binding(1, 0)]]
SamplerState smp;

struct Fragment {
    float4 col: SV_Target0;
}

[shader("fragment")]
Fragment fragment_main(Vertex vertex) {
    return { tex.Sample(smp, vertex.uv) };
}
