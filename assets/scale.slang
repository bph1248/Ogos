struct Vertex {
    float4 pos: SV_Position;
    float2 uv: TEXCOORD0;
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

struct Fragment {
    float4 col: SV_Target0;
}

struct PushConstants {
    RenderPassKind render_pass_kind;
    float2 src_extent;
    float2 dst_extent;
}

enum FilterKind: uint32_t {
    NEAREST_NEIGHBOR,
    BILINEAR,
    BLACKMAN
}

enum RenderPassKind: uint32_t {
    HORIZONTAL,
    VERTICAL
};

float blackman(float x) {
    let t = abs(x);

    if (t >= KERNEL_SUPPORT) {
        return 0.0;
    }

    let pi_t = float.getPi() * t;
    let window = 0.42 +
        0.5 * cos(pi_t / KERNEL_SUPPORT) +
        0.08 * cos(2.0 * pi_t / KERNEL_SUPPORT);

    return sinc(t) * window;
}

float sinc(float x) {
    if (abs(x) < 1e-6) {
        return 1.0;
    }

    let pi_x = float.getPi() * x;

    return sin(pi_x) / pi_x;
}

static const float KERNEL_SUPPORT = 3.0;

[vk::constant_id(0)]
const FilterKind FILTER_KIND;

[vk::push_constant]
ConstantBuffer<PushConstants> push_consts;

[[vk::binding(0, 0)]]
Texture2D tex;
[[vk::binding(1, 0)]]
SamplerState sampler;

[shader("fragment")]
Fragment fragment_main(Vertex vertex) {
    var fragment = Fragment(float4(0.0));
    switch (FILTER_KIND) {
        case FilterKind::NEAREST_NEIGHBOR:
        case FilterKind::BILINEAR:
            fragment = { tex.Sample(sampler, vertex.uv) };
            break;
        case FilterKind::BLACKMAN:
            let src_pos = vertex.uv * push_consts.src_extent;
            let src_texel = int2(floor(src_pos));
            let src_dist = src_texel + 0.5 - src_pos;

            var dst_scale = 0.0;
            switch (push_consts.render_pass_kind) {
                case RenderPassKind::HORIZONTAL:
                    dst_scale = push_consts.dst_extent.x / push_consts.src_extent.x;
                    break;
                case RenderPassKind::VERTICAL:
                    dst_scale = push_consts.dst_extent.y / push_consts.src_extent.y;
            }
            let kernel_scale = min(dst_scale, 1.0);
            let radius = int(ceil(KERNEL_SUPPORT / kernel_scale));

            var weighted_sample_acc = float4(0.0);
            var weight_acc = 0.0;

            for (var tap_i = -radius; tap_i <= radius; ++tap_i) {
                var tap_texel = int2(0);
                var tap_dist = 0.0;
                switch (push_consts.render_pass_kind) {
                    case RenderPassKind::HORIZONTAL:
                        tap_texel = src_texel + int2(tap_i, 0);
                        tap_dist = src_dist.x + tap_i;
                        break;
                    case RenderPassKind::VERTICAL:
                        tap_texel = src_texel + int2(0, tap_i);
                        tap_dist = src_dist.y + tap_i;
                }
                let weight = blackman(kernel_scale * tap_dist);
                let tap_texel_clamped = clamp(tap_texel, int2(0, 0), int2(push_consts.src_extent - 1));
                let sample = tex.Load(int3(tap_texel_clamped, 0));

                weighted_sample_acc += sample * weight;
                weight_acc += weight;
            }

            fragment = { weighted_sample_acc / weight_acc };
    }

    return fragment;
}
