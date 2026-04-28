// Instanced compositing for legacy-correct per-entry blur textures packed into an atlas.

struct FrameUniform {
    viewport_size: vec2<f32>,
    _pad: vec2<f32>,
}

struct BlurInstance {
    rect: vec4<f32>,        // screen x, y, w, h in pixels
    atlas_rect: vec4<f32>,  // atlas x, y, w, h in pixels
    color: vec4<f32>,       // overlay rgba
    params: vec4<f32>,      // border_radius, opacity, dark_mode, unused
}

@group(0) @binding(0) var t_blur: texture_2d<f32>;
@group(0) @binding(1) var s_blur: sampler;
@group(0) @binding(2) var<uniform> frame: FrameUniform;
@group(0) @binding(3) var<storage, read> instances: array<BlurInstance>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
    @location(1) atlas_px: vec2<f32>,
    @location(2) size: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) params: vec4<f32>,
}

fn unit_pos(index: u32) -> vec2<f32> {
    switch index {
        case 0u: { return vec2<f32>(0.0, 0.0); }
        case 1u: { return vec2<f32>(1.0, 0.0); }
        case 2u: { return vec2<f32>(0.0, 1.0); }
        case 3u: { return vec2<f32>(1.0, 0.0); }
        case 4u: { return vec2<f32>(1.0, 1.0); }
        case 5u: { return vec2<f32>(0.0, 1.0); }
        default: { return vec2<f32>(0.0, 0.0); }
    }
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32,
           @builtin(instance_index) instance_index: u32) -> VertexOutput {
    let inst = instances[instance_index];
    let uv = unit_pos(vertex_index);
    let screen = inst.rect.xy + uv * inst.rect.zw;
    let clip = vec2<f32>(
        screen.x / frame.viewport_size.x * 2.0 - 1.0,
        1.0 - screen.y / frame.viewport_size.y * 2.0
    );

    var out: VertexOutput;
    out.position = vec4<f32>(clip, 0.0, 1.0);
    out.local_pos = uv * inst.rect.zw;
    out.atlas_px = inst.atlas_rect.xy + uv * inst.atlas_rect.zw;
    out.size = inst.rect.zw;
    out.color = inst.color;
    out.params = inst.params;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let dims_u = textureDimensions(t_blur);
    let dims = vec2<f32>(f32(dims_u.x), f32(dims_u.y));
    let blur_color = textureSample(t_blur, s_blur, (in.atlas_px + vec2<f32>(0.5, 0.5)) / dims);

    let radius = in.params.x;
    var corner_alpha = 1.0;
    if (radius > 0.0) {
        let corner_radius = min(radius, min(in.size.x, in.size.y) * 0.5);
        let d = abs(in.local_pos - in.size * 0.5) - (in.size * 0.5 - vec2<f32>(corner_radius, corner_radius));
        let dist = length(max(d, vec2<f32>(0.0, 0.0))) + min(max(d.x, d.y), 0.0);
        corner_alpha = 1.0 - smoothstep(corner_radius - 1.0, corner_radius, dist);
    }

    let tint = in.color.rgb;
    let final_rgb = blur_color.rgb * (1.0 - in.color.a) + tint * in.color.a;
    let final_alpha = blur_color.a * corner_alpha * in.params.y;
    return vec4<f32>(final_rgb * final_alpha, final_alpha);
}
