// Blur texture compositing shader with frosted glass overlay support
// Draws a blurred texture at a specific position with proper transform
// Then overlays a color with rounded corners for the glass effect

struct Uniforms {
    // Transform matrix stored as 3 vec4s (for alignment)
    // Row 0: [m00, m01, 0, 0] - x basis vector + padding
    // Row 1: [m10, m11, 0, 0] - y basis vector + padding
    // Row 2: [tx,  ty, opacity, padding]
    m00: f32, m01: f32, _pad0: f32, _pad1: f32,
    m10: f32, m11: f32, _pad2: f32, _pad3: f32,
    tx:  f32, ty:  f32, opacity: f32, _pad4: f32,
}

struct OverlayUniforms {
    // Overlay color (RGBA, premultiplied)
    color_r: f32,
    color_g: f32,
    color_b: f32,
    color_a: f32,
    // Border radius and view size
    border_radius: f32,
    view_width: f32,
    view_height: f32,
    _pad: f32,
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
    // Generate quad vertices (0,0), (1,0), (0,1), (1,1)
    let x = f32(index % 2u);
    let y = f32(index / 2u);

    // Build transform matrix
    let m00 = uniforms.m00;
    let m01 = uniforms.m01;
    let m10 = uniforms.m10;
    let m11 = uniforms.m11;
    let tx = uniforms.tx;
    let ty = uniforms.ty;

    // Apply affine transform: pos' = M * pos + T
    let pos_x = x;
    let pos_y = y;
    let transformed_x = m00 * pos_x + m01 * pos_y + tx;
    let transformed_y = m10 * pos_x + m11 * pos_y + ty;

    var out: VertexOutput;
    out.position = vec4<f32>(transformed_x, transformed_y, 0.0, 1.0);
    out.uv = vec2<f32>(x, 1.0 - y); // Flip Y for texture coordinates
    out.local_pos = vec2<f32>(x * overlay.view_width, y * overlay.view_height);
    return out;
}

// Calculate rounded rectangle alpha
fn rounded_rect_alpha(pos: vec2<f32>, size: vec2<f32>, radius: f32) -> f32 {
    if radius <= 0.0 {
        return 1.0;
    }

    // Adjust position to be relative to the nearest corner
    let corner_radius = min(radius, min(size.x, size.y) * 0.5);

    // Distance from nearest inner corner center
    let d = abs(pos - size * 0.5) - (size * 0.5 - vec2<f32>(corner_radius, corner_radius));
    let dist = length(max(d, vec2<f32>(0.0, 0.0))) + min(max(d.x, d.y), 0.0);

    // Smooth step for anti-aliased edge
    return 1.0 - smoothstep(corner_radius - 1.0, corner_radius, dist);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Sample the blurred background
    let blur_color = textureSample(t_blur, s_blur, in.uv);

    // Apply opacity to blur
    let blurred = vec4<f32>(blur_color.rgb, blur_color.a * uniforms.opacity);

    // Calculate overlay color with rounded corners
    let view_size = vec2<f32>(overlay.view_width, overlay.view_height);
    let corner_alpha = rounded_rect_alpha(in.local_pos, view_size, overlay.border_radius);

    // Premultiplied overlay color
    let overlay_rgba = vec4<f32>(
        overlay.color_r,
        overlay.color_g,
        overlay.color_b,
        overlay.color_a * uniforms.opacity * corner_alpha
    );

    // Composite overlay on top of blurred background
    // Standard alpha blending: result = src + dst * (1 - src_alpha)
    let final_rgb = overlay_rgba.rgb + blurred.rgb * (1.0 - overlay_rgba.a);
    let final_alpha = overlay_rgba.a + blurred.a * (1.0 - overlay_rgba.a);

    return vec4<f32>(final_rgb, final_alpha);
}
