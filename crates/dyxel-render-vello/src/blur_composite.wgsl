// Blur texture compositing shader

struct Uniforms {
    m00: f32, m01: f32, _pad0: f32, _pad1: f32,
    m10: f32, m11: f32, _pad2: f32, _pad3: f32,
    tx:  f32, ty:  f32, opacity: f32, _pad4: f32,
}

struct OverlayUniforms {
    color_r: f32,
    color_g: f32,
    color_b: f32,
    color_a: f32,
    border_radius: f32,
    view_width: f32,
    view_height: f32,
    color_mode: f32,
}

@group(0) @binding(0) var t_blur: texture_2d<f32>;
@group(0) @binding(1) var s_blur: sampler;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;
@group(0) @binding(3) var<uniform> overlay: OverlayUniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) local_pos: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    var pos: vec2<f32>;
    switch index {
        case 0u: { pos = vec2<f32>(-1.0, -1.0); }
        case 1u: { pos = vec2<f32>( 1.0, -1.0); }
        case 2u: { pos = vec2<f32>(-1.0,  1.0); }
        case 3u: { pos = vec2<f32>( 1.0, -1.0); }
        case 4u: { pos = vec2<f32>( 1.0,  1.0); }
        case 5u: { pos = vec2<f32>(-1.0,  1.0); }
        default: { pos = vec2<f32>(0.0, 0.0); }
    }

    var out: VertexOutput;
    let uv = vec2<f32>(pos.x * 0.5 + 0.5, -pos.y * 0.5 + 0.5);
    let clip_x = uniforms.m00 * uv.x + uniforms.m01 * uv.y + uniforms.tx;
    let clip_y = uniforms.m10 * uv.x + uniforms.m11 * uv.y + uniforms.ty;
    out.position = vec4<f32>(clip_x, clip_y, 0.0, 1.0);
    out.uv = uv;
    out.local_pos = vec2<f32>(uv.x * overlay.view_width, uv.y * overlay.view_height);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Sample the blur texture
    let blur_color = textureSample(t_blur, s_blur, in.uv);

    // Apply rounded corner alpha
    let size = vec2<f32>(overlay.view_width, overlay.view_height);
    let radius = overlay.border_radius;
    var corner_alpha: f32 = 1.0;
    if (radius > 0.0) {
        let corner_radius = min(radius, min(size.x, size.y) * 0.5);
        let d = abs(in.local_pos - size * 0.5) - (size * 0.5 - vec2<f32>(corner_radius, corner_radius));
        let dist = length(max(d, vec2<f32>(0.0, 0.0))) + min(max(d.x, d.y), 0.0);
        corner_alpha = 1.0 - smoothstep(corner_radius - 1.0, corner_radius, dist);
    }

    // Apply tint
    let tint = vec3<f32>(overlay.color_r, overlay.color_g, overlay.color_b);
    let final_rgb = blur_color.rgb * (1.0 - overlay.color_a) + tint * overlay.color_a;
    let final_alpha = blur_color.a * corner_alpha * uniforms.opacity;

    // Return premultiplied alpha
    return vec4<f32>(final_rgb * final_alpha, final_alpha);
}
