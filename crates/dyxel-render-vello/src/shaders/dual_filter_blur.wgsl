// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Dual Filtering (Kawase-style) Blur Compute Shader
//!
//! Optimized blur using iterative downsample/upsample passes.
//! Much faster than traditional Gaussian blur for large radii.
//! Reference: https://community.arm.com/cfs-file/__key/communityserver-blogs-components-weblogfiles/00-00-00-20-66/siggraph2015_2D00_mmg_5F00_marius.pptx

struct BlurParams {
    // Current pass index (0 = first downsample)
    pass_index: u32,
    // Total number of passes
    pass_count: u32,
    // Direction: 0 = downsample, 1 = upsample
    direction: u32,
    // Padding to 16 bytes
    _padding: u32,
    // Input texture size
    input_size: vec2<f32>,
    // Output texture size
    output_size: vec2<f32>,
}

@group(0) @binding(0) var input_texture: texture_2d<f32>;
@group(0) @binding(1) var output_texture: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(2) var<uniform> params: BlurParams;

// Sample with bilinear interpolation using textureLoad at fractional coordinates
fn sample_bilinear(tex: texture_2d<f32>, uv: vec2<f32>, tex_size: vec2<f32>) -> vec4<f32> {
    let coord = uv * tex_size - vec2<f32>(0.5);
    let base_coord = vec2<i32>(floor(coord));
    let frac = fract(coord);

    // Load 4 samples
    let s00 = textureLoad(tex, clamp(base_coord + vec2<i32>(0, 0), vec2<i32>(0), vec2<i32>(tex_size) - vec2<i32>(1)), 0);
    let s10 = textureLoad(tex, clamp(base_coord + vec2<i32>(1, 0), vec2<i32>(0), vec2<i32>(tex_size) - vec2<i32>(1)), 0);
    let s01 = textureLoad(tex, clamp(base_coord + vec2<i32>(0, 1), vec2<i32>(0), vec2<i32>(tex_size) - vec2<i32>(1)), 0);
    let s11 = textureLoad(tex, clamp(base_coord + vec2<i32>(1, 1), vec2<i32>(0), vec2<i32>(tex_size) - vec2<i32>(1)), 0);

    // Bilinear interpolation
    let s0 = mix(s00, s10, frac.x);
    let s1 = mix(s01, s11, frac.x);
    return mix(s0, s1, frac.y);
}

@compute @workgroup_size(8, 8)
fn blur_main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let output_size = vec2<i32>(params.output_size);
    let coord = vec2<i32>(global_id.xy);

    // Check bounds
    if (coord.x >= output_size.x || coord.y >= output_size.y) {
        return;
    }

    // Calculate UV coordinates
    let uv = (vec2<f32>(coord) + vec2<f32>(0.5)) / params.output_size;

    // Calculate texel size for current input
    let input_texel_size = vec2<f32>(1.0) / params.input_size;

    var result: vec4<f32>;

    if (params.direction == 0u) {
        // Downsample pass
        // Use larger offset for early passes to achieve bigger blur
        let scale = 1.0 + f32(params.pass_index) * 0.5;
        let offset = input_texel_size * scale;

        // 4-sample box filter with offset (simulating bilinear sampling)
        result = vec4<f32>(0.0);
        result += sample_bilinear(input_texture, uv + vec2<f32>(-offset.x, -offset.y), params.input_size);
        result += sample_bilinear(input_texture, uv + vec2<f32>( offset.x, -offset.y), params.input_size);
        result += sample_bilinear(input_texture, uv + vec2<f32>(-offset.x,  offset.y), params.input_size);
        result += sample_bilinear(input_texture, uv + vec2<f32>( offset.x,  offset.y), params.input_size);
        result *= 0.25;
    } else {
        // Upsample pass
        let offset = input_texel_size;
        result = vec4<f32>(0.0);
        result += sample_bilinear(input_texture, uv + vec2<f32>(-offset.x, -offset.y), params.input_size) * 0.25;
        result += sample_bilinear(input_texture, uv + vec2<f32>( offset.x, -offset.y), params.input_size) * 0.25;
        result += sample_bilinear(input_texture, uv + vec2<f32>(-offset.x,  offset.y), params.input_size) * 0.25;
        result += sample_bilinear(input_texture, uv + vec2<f32>( offset.x,  offset.y), params.input_size) * 0.25;
    }

    textureStore(output_texture, coord, result);
}
