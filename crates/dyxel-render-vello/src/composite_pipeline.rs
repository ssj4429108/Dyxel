// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Composite Pipeline - GPU-accelerated layer compositing
//!
//! Implements efficient compositing of offscreen layer textures onto the main render target.
//! Supports alpha blending, blend modes, and proper coordinate transformation.

use std::sync::Arc;

/// Uniforms for layer compositing
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CompositeUniforms {
    /// Position (x, y) and size (width, height) in screen pixels
    pub rect: [f32; 4],
    /// Screen size (width, height)
    pub screen_size: [f32; 2],
    /// Alpha value (0.0 - 1.0)
    pub alpha: f32,
    /// Blend mode: 0=Normal, 1=Multiply, 2=Screen, etc.
    pub blend_mode: u32,
}

impl CompositeUniforms {
    /// Create new composite uniforms
    pub fn new(
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        screen_width: f32,
        screen_height: f32,
        alpha: f32,
        blend_mode: u32,
    ) -> Self {
        Self {
            rect: [x, y, width, height],
            screen_size: [screen_width, screen_height],
            alpha,
            blend_mode,
        }
    }
}

/// GPU composite pipeline for layer blending
pub struct CompositePipeline {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,

    /// Render pipeline for compositing
    pipeline: wgpu::RenderPipeline,
    /// Bind group layout
    bind_group_layout: wgpu::BindGroupLayout,
    /// Sampler for texture sampling
    sampler: wgpu::Sampler,
    /// Uniform buffer (reused for all composite operations)
    uniform_buffer: wgpu::Buffer,
}

impl CompositePipeline {
    /// Create a new composite pipeline
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Result<Self, String> {
        // Create bind group layout
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Composite Bind Group Layout"),
            entries: &[
                // Source texture
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // Sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // Uniforms
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        // Create pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Composite Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // Load shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Composite Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/layer_composite.wgsl").into()),
        });

        // Create render pipeline
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Composite Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // Create sampler
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Composite Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // Create uniform buffer
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Composite Uniform Buffer"),
            size: std::mem::size_of::<CompositeUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            device,
            queue,
            pipeline,
            bind_group_layout,
            sampler,
            uniform_buffer,
        })
    }

    /// Composite a source texture onto a render target
    ///
    /// # Arguments
    /// * `encoder` - Command encoder
    /// * `source_texture` - Source texture to composite
    /// * `target_view` - Target render attachment
    /// * `x` - X position in pixels (top-left)
    /// * `y` - Y position in pixels (top-left)
    /// * `width` - Width in pixels
    /// * `height` - Height in pixels
    /// * `screen_width` - Screen width in pixels
    /// * `screen_height` - Screen height in pixels
    /// * `alpha` - Alpha multiplier (0.0 - 1.0)
    /// * `blend_mode` - Blend mode (0 = Normal)
    pub fn composite(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source_texture: &wgpu::TextureView,
        target_view: &wgpu::TextureView,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        screen_width: f32,
        screen_height: f32,
        alpha: f32,
        blend_mode: u32,
    ) {
        // Update uniforms
        let uniforms = CompositeUniforms::new(
            x, y, width, height,
            screen_width, screen_height,
            alpha, blend_mode,
        );

        self.queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::bytes_of(&uniforms),
        );

        // Create bind group for this composite operation
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Composite Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source_texture),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
            ],
        });

        // Begin render pass
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Composite Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load, // Preserve existing content
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, &bind_group, &[]);
            render_pass.draw(0..6, 0..1); // Quad (2 triangles)
        }
    }

    /// Composite with clear (for first layer)
    pub fn composite_with_clear(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source_texture: &wgpu::TextureView,
        target_view: &wgpu::TextureView,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        screen_width: f32,
        screen_height: f32,
        alpha: f32,
        blend_mode: u32,
        clear_color: wgpu::Color,
    ) {
        // Update uniforms
        let uniforms = CompositeUniforms::new(
            x, y, width, height,
            screen_width, screen_height,
            alpha, blend_mode,
        );

        self.queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::bytes_of(&uniforms),
        );

        // Create bind group for this composite operation
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Composite Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source_texture),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
            ],
        });

        // Begin render pass with clear
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Composite Render Pass (Clear)"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear_color),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, &bind_group, &[]);
            render_pass.draw(0..6, 0..1); // Quad (2 triangles)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_composite_uniforms() {
        let uniforms = CompositeUniforms::new(
            10.0, 20.0, 100.0, 200.0,
            1920.0, 1080.0,
            0.5, 0,
        );

        assert_eq!(uniforms.rect, [10.0, 20.0, 100.0, 200.0]);
        assert_eq!(uniforms.screen_size, [1920.0, 1080.0]);
        assert_eq!(uniforms.alpha, 0.5);
        assert_eq!(uniforms.blend_mode, 0);
    }
}
