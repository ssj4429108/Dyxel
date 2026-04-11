// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Kawase Blur - Render Pipeline Variant
//!
//! Three-mode shader for Dual Kawase large-radius blur:
//!   mode=0: Downsample (4-tap box filter with pre-blur)
//!   mode=1: Kawase blur (8-tap, offset grows with pass_index)
//!   mode=2: Upsample (4-tap bilinear reconstruction)
//!
//! Uses render pipeline (not compute) so output can be Rgba16Float,
//! preventing rounding errors from multi-pass accumulation.

struct KawaseParams {
    // 0=downsample, 1=kawase, 2=upsample
    mode: u32,
    // Current pass index (used by kawase mode to scale offset)
    pass_index: u32,
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(0) var t_source: texture_2d<f32>;
@group(0) @binding(1) var s_source: sampler;
@group(0) @binding(2) var<uniform> params: KawaseParams;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    // Full-screen triangle (3 vertices cover the entire NDC space)
    let x = f32(i32(index & 1u) << 2u) - 1.0;
    let y = f32(i32(index & 2u) << 1u) - 1.0;
    var out: VertexOutput;
    out.clip_position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>(x * 0.5 + 0.5, 0.5 - y * 0.5);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_size = vec2<f32>(textureDimensions(t_source));
    let texel = 1.0 / tex_size;
    let uv = in.uv;

    var result: vec4<f32>;

    if (params.mode == 0u) {
        // Downsample: 4-tap box at ±1 texel offset
        // Fixed 1.0 offset (not scale-with-pass) for clean pre-blur that suppresses aliasing
        let off = texel * 1.0;
        result = (
            textureSample(t_source, s_source, uv + vec2<f32>(-off.x, -off.y)) +
            textureSample(t_source, s_source, uv + vec2<f32>( off.x, -off.y)) +
            textureSample(t_source, s_source, uv + vec2<f32>(-off.x,  off.y)) +
            textureSample(t_source, s_source, uv + vec2<f32>( off.x,  off.y))
        ) * 0.25;
    } else if (params.mode == 1u) {
        // Kawase blur: 8-tap, half-texel offset scaled by pass_index
        // (pass_index + 0.5) samples between pixel centers for natural bilinear smoothing
        let half = texel * (f32(params.pass_index) + 0.5);
        result =
            textureSample(t_source, s_source, uv + vec2<f32>(-half.x, -half.y)) * 0.125 +
            textureSample(t_source, s_source, uv + vec2<f32>( half.x, -half.y)) * 0.125 +
            textureSample(t_source, s_source, uv + vec2<f32>(-half.x,  half.y)) * 0.125 +
            textureSample(t_source, s_source, uv + vec2<f32>( half.x,  half.y)) * 0.125 +
            textureSample(t_source, s_source, uv + vec2<f32>(-half.x * 2.0, 0.0)) * 0.125 +
            textureSample(t_source, s_source, uv + vec2<f32>( half.x * 2.0, 0.0)) * 0.125 +
            textureSample(t_source, s_source, uv + vec2<f32>(0.0, -half.y * 2.0)) * 0.125 +
            textureSample(t_source, s_source, uv + vec2<f32>(0.0,  half.y * 2.0)) * 0.125;
    } else {
        // Upsample: 4-tap bilinear at ±1 texel offset for smooth reconstruction
        let off = texel;
        result = (
            textureSample(t_source, s_source, uv + vec2<f32>(-off.x, -off.y)) +
            textureSample(t_source, s_source, uv + vec2<f32>( off.x, -off.y)) +
            textureSample(t_source, s_source, uv + vec2<f32>(-off.x,  off.y)) +
            textureSample(t_source, s_source, uv + vec2<f32>( off.x,  off.y))
        ) * 0.25;
    }

    return result;
}
