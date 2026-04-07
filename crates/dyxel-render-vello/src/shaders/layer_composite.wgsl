// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Layer Composite Shader
//!
//! Used to composite offscreen layer textures onto the main render target.
//! Supports alpha blending and blend modes.

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
};

// Uniforms for layer transform and alpha
struct LayerUniforms {
    // Position (x, y) and size (width, height) in screen pixels
    rect: vec4<f32>,
    // Screen size (width, height)
    screen_size: vec2<f32>,
    // Alpha value (0.0 - 1.0)
    alpha: f32,
    // Blend mode: 0=Normal, 1=Multiply, 2=Screen, etc.
    blend_mode: u32,
};

@group(0) @binding(0) var layer_texture: texture_2d<f32>;
@group(0) @binding(1) var layer_sampler: sampler;
@group(0) @binding(2) var<uniform> uniforms: LayerUniforms;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;

    // Generate a quad covering the layer bounds
    // vertex_index: 0=top-left, 1=top-right, 2=bottom-left, 3=bottom-left, 4=top-right, 5=bottom-right

    let rect = uniforms.rect;
    let screen_w = uniforms.screen_size.x;
    let screen_h = uniforms.screen_size.y;

    // Layer bounds in pixels (top-left origin)
    let layer_x = rect.x;
    let layer_y = rect.y;
    let layer_w = rect.z;
    let layer_h = rect.w;

    // Calculate corner positions based on vertex index
    var pos: vec2<f32>;
    var uv: vec2<f32>;

    switch vertex_index {
        case 0u: { // Top-left
            pos = vec2<f32>(layer_x, layer_y);
            uv = vec2<f32>(0.0, 0.0);
        }
        case 1u: { // Top-right
            pos = vec2<f32>(layer_x + layer_w, layer_y);
            uv = vec2<f32>(1.0, 0.0);
        }
        case 2u: { // Bottom-left
            pos = vec2<f32>(layer_x, layer_y + layer_h);
            uv = vec2<f32>(0.0, 1.0);
        }
        case 3u: { // Bottom-left (duplicate for triangle 2)
            pos = vec2<f32>(layer_x, layer_y + layer_h);
            uv = vec2<f32>(0.0, 1.0);
        }
        case 4u: { // Top-right (duplicate for triangle 2)
            pos = vec2<f32>(layer_x + layer_w, layer_y);
            uv = vec2<f32>(1.0, 0.0);
        }
        case 5u: { // Bottom-right
            pos = vec2<f32>(layer_x + layer_w, layer_y + layer_h);
            uv = vec2<f32>(1.0, 1.0);
        }
        default: {
            pos = vec2<f32>(0.0, 0.0);
            uv = vec2<f32>(0.0, 0.0);
        }
    }

    // Convert pixel coordinates to NDC (-1 to 1)
    // NDC: (-1, -1) is bottom-left, (1, 1) is top-right
    // But our pos is top-left origin, so we need to flip Y
    let ndc_x = (pos.x / screen_w) * 2.0 - 1.0;
    let ndc_y = 1.0 - (pos.y / screen_h) * 2.0;

    out.position = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.tex_coord = uv;

    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_color = textureSample(layer_texture, layer_sampler, in.tex_coord);

    // Apply layer alpha to texture color
    // Output non-premultiplied RGBA, blend state will handle the mixing
    let final_alpha = tex_color.a * uniforms.alpha;

    // Blend mode handling
    switch uniforms.blend_mode {
        case 0u: {  // Normal
            // Return non-premultiplied color with applied alpha
            // The blend state (ALPHA_BLENDING) will do: dst * (1 - src_a) + src * src_a
            return vec4<f32>(tex_color.rgb, final_alpha);
        }
        case 1u: {  // Multiply
            // For now, same as normal - proper multiply requires reading dst
            return vec4<f32>(tex_color.rgb, final_alpha);
        }
        case 2u: {  // Screen
            // For now, same as normal
            return vec4<f32>(tex_color.rgb, final_alpha);
        }
        default: {
            return vec4<f32>(tex_color.rgb, final_alpha);
        }
    }
}
