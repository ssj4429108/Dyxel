// Blur texture compositing shader
// Draws a blurred texture at a specific position with proper transform

struct Uniforms {
    // Transform matrix stored as 3 vec4s (for alignment)
    // Row 0: [m00, m01, 0, 0] - x basis vector + padding
    // Row 1: [m10, m11, 0, 0] - y basis vector + padding
    // Row 2: [tx,  ty,  opacity, padding]
    m00: f32, m01: f32, _pad0: f32, _pad1: f32,
    m10: f32, m11: f32, _pad2: f32, _pad3: f32,
    tx:  f32, ty:  f32, opacity: f32, _pad4: f32,
}

@group(0) @binding(0) var t_blur: texture_2d<f32>;
@group(0) @binding(1) var s_blur: sampler;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
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
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(t_blur, s_blur, in.uv);
    return vec4<f32>(color.rgb, color.a * uniforms.opacity);
}
