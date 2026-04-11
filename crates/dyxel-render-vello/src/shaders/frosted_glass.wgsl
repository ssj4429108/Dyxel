// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Frosted Glass Effect - Dual-Pass Separable Gaussian Blur
//!
//! Pass 1: Horizontal blur (direction = [1.0, 0.0])
//! Pass 2: Vertical blur + Tinting + Noise (direction = [0.0, 1.0])

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    // Generate a full-screen triangle
    let x = f32(i32(index & 1u) << 2u) - 1.0;
    let y = f32(i32(index & 2u) << 1u) - 1.0;
    var out: VertexOutput;
    out.clip_position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>(x * 0.5 + 0.5, 0.5 - y * 0.5);
    return out;
}

struct BlurParams {
    direction: vec2<f32>,  // Pass 1: [1.0, 0.0], Pass 2: [0.0, 1.0]
    radius: f32,
    noise_strength: f32,
    padding: f32,
    tint_color: vec4<f32>,
}

@group(0) @binding(0) var t_source: texture_2d<f32>;
@group(0) @binding(1) var s_source: sampler;
@group(0) @binding(2) var<uniform> params: BlurParams;

// Pre-computed Gaussian weights for 5-sample kernel
// weights[0] = center, weights[1..4] = offsets
const weights = array<f32, 5>(0.227027, 0.1945946, 0.1216216, 0.054054, 0.016216);

// Simple pseudo-random function for noise
fn random(uv: vec2<f32>) -> f32 {
    return fract(sin(dot(uv, vec2<f32>(12.9898, 78.233))) * 43758.5453123);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_size = vec2<f32>(textureDimensions(t_source));
    let texel_size = 1.0 / tex_size;

    // Sample center for alpha
    let center_sample = textureSample(t_source, s_source, in.uv);
    var result = center_sample.rgb * weights[0];
    var total_alpha = center_sample.a * weights[0];

    // One-dimensional Gaussian blur
    for (var i = 1; i < 5; i++) {
        let offset = params.direction * f32(i) * params.radius * texel_size;
        let weight = weights[i];

        let sample1 = textureSample(t_source, s_source, in.uv + offset);
        let sample2 = textureSample(t_source, s_source, in.uv - offset);

        result += sample1.rgb * weight;
        result += sample2.rgb * weight;
        total_alpha += sample1.a * weight;
        total_alpha += sample2.a * weight;
    }

    // Note: Color tinting and noise are now handled by blur_composite.wgsl
    // This shader only performs the dual-pass Gaussian blur

    return vec4<f32>(result, total_alpha);
}
