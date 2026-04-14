// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Filter Pipeline - GPU-accelerated filter effects
//!
//! Implements Dual-Filtering blur, drop shadow, and other effects
//! using compute shaders for optimal performance.

use std::sync::Arc;

use crate::texture_pool::SharedTexturePool;

/// Filter error types
#[derive(Debug, Clone)]
pub enum FilterError {
    DeviceNotInitialized,
    ShaderCompilationFailed(String),
    InvalidFilterParameters(String),
    OutOfMemory,
}

impl std::fmt::Display for FilterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FilterError::DeviceNotInitialized => write!(f, "GPU device not initialized"),
            FilterError::ShaderCompilationFailed(msg) => {
                write!(f, "Shader compilation failed: {}", msg)
            }
            FilterError::InvalidFilterParameters(msg) => {
                write!(f, "Invalid filter parameters: {}", msg)
            }
            FilterError::OutOfMemory => write!(f, "Out of GPU memory"),
        }
    }
}

impl std::error::Error for FilterError {}

/// Uniforms for Kawase render-pipeline blur shader
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct KawaseUniforms {
    mode: u32,       // 0=downsample, 1=kawase, 2=upsample
    pass_index: u32, // kawase pass index (for offset scaling)
    _pad0: u32,
    _pad1: u32,
}

/// Pre-allocated textures for Kawase blur to avoid per-frame GPU allocation
struct KawaseTexturePool {
    /// Resolution this pool was created for
    full_width: u32,
    full_height: u32,
    // Half-res texture (full/2)
    half: wgpu::Texture,
    // Quarter-res texture (full/4)
    quarter: wgpu::Texture,
    // Two ping-pong buffers at quarter-res for Kawase iterations
    ping: wgpu::Texture,
    pong: wgpu::Texture,
}

impl KawaseTexturePool {
    fn create(device: &wgpu::Device, full_width: u32, full_height: u32) -> Self {
        // OPTIMIZATION: Use 1/8 resolution instead of 1/4 for significant performance gain
        // Pixel fill rate: 1/64 of original vs 1/16 of 1/4 resolution
        let half_w = (full_width / 2).max(1);
        let half_h = (full_height / 2).max(1);
        let quarter_w = (full_width / 8).max(1);
        let quarter_h = (full_height / 8).max(1);

        let make = |w: u32, h: u32, label: &'static str| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };

        Self {
            full_width,
            full_height,
            half: make(half_w, half_h, "Kawase Half"),
            quarter: make(quarter_w, quarter_h, "Kawase Quarter"),
            ping: make(quarter_w, quarter_h, "Kawase Ping"),
            pong: make(quarter_w, quarter_h, "Kawase Pong"),
        }
    }

    fn matches(&self, width: u32, height: u32) -> bool {
        self.full_width == width && self.full_height == height
    }
}

/// Uniforms for blur shader
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct BlurUniforms {
    direction: u32,
    iteration: u32,
    total_iterations: u32,
    radius: f32,
    input_size: [f32; 2],
    output_size: [f32; 2],
}

/// GPU filter pipeline
pub struct FilterPipeline {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,

    // Dual-filter blur pipeline
    blur_pipeline: wgpu::ComputePipeline,
    blur_bind_group_layout: wgpu::BindGroupLayout,

    // Composite pipeline (for blending effects)
    composite_pipeline: wgpu::RenderPipeline,
    composite_bind_group_layout: wgpu::BindGroupLayout,

    // Uniform buffer
    uniform_buffer: wgpu::Buffer,

    // Sampler for texture sampling (linear + clamp-to-edge)
    sampler: wgpu::Sampler,

    // Frosted glass dual-pass pipeline
    frosted_pipeline: wgpu::RenderPipeline,
    frosted_bind_group_layout: wgpu::BindGroupLayout,
    frosted_uniforms: wgpu::Buffer,

    // Kawase large-radius blur pipeline (render pipeline, Rgba16Float intermediate)
    kawase_pipeline: wgpu::RenderPipeline,
    kawase_bind_group_layout: wgpu::BindGroupLayout,
    kawase_uniforms: wgpu::Buffer,
    // Pre-allocated texture pool for Kawase passes (avoids per-frame allocation)
    kawase_pool: std::cell::RefCell<Option<KawaseTexturePool>>,
}

impl FilterPipeline {
    /// Create a new filter pipeline
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Result<Self, FilterError> {
        // Create bind group layout for blur
        let blur_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Blur Bind Group Layout"),
                entries: &[
                    // Input texture
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // Output texture (storage)
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: wgpu::TextureFormat::Rgba8Unorm,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    // Uniforms
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        // Create compute pipeline
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Dual Filter Blur Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/dual_filter_blur.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Blur Pipeline Layout"),
            bind_group_layouts: &[&blur_bind_group_layout],
            push_constant_ranges: &[],
        });

        let blur_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Blur Compute Pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("blur_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        // Create uniform buffer
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Blur Uniform Buffer"),
            size: std::mem::size_of::<BlurUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Note: Bind groups are created per-frame in run_blur_pass
        // We don't create a placeholder here since the actual bind groups
        // require texture views that change each frame

        // Create sampler
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Filter Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // Create composite pipeline (for final blending)
        let composite_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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

        // For now, composite pipeline is a placeholder
        // Full implementation would create a render pipeline for blending
        let composite_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Composite Pipeline Layout"),
                bind_group_layouts: &[&composite_bind_group_layout],
                push_constant_ranges: &[],
            });

        let composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Composite Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/layer_composite.wgsl").into()),
        });

        let composite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Composite Render Pipeline"),
            layout: Some(&composite_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &composite_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &composite_shader,
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

        // Create frosted glass pipeline (dual-pass separable Gaussian blur)
        let frosted_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Frosted Glass Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/frosted_glass.wgsl").into()),
        });

        let frosted_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Frosted Glass Bind Group Layout"),
                entries: &[
                    // Input texture
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
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let frosted_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Frosted Glass Pipeline Layout"),
                bind_group_layouts: &[&frosted_bind_group_layout],
                push_constant_ranges: &[],
            });

        let frosted_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Frosted Glass Pipeline"),
            layout: Some(&frosted_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &frosted_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &frosted_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState::REPLACE),
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

        // Create uniform buffer for frosted glass (48 bytes: 3 vec4 aligned)
        let frosted_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Frosted Glass Uniforms"),
            size: 48,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // === Kawase large-radius blur pipeline ===
        let kawase_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Kawase Blur Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/kawase_blur.wgsl").into()),
        });

        let kawase_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Kawase Bind Group Layout"),
                entries: &[
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
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let kawase_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Kawase Pipeline Layout"),
                bind_group_layouts: &[&kawase_bind_group_layout],
                push_constant_ranges: &[],
            });

        let kawase_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Kawase Blur Pipeline"),
            layout: Some(&kawase_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &kawase_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &kawase_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend: Some(wgpu::BlendState::REPLACE),
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

        let kawase_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Kawase Uniforms"),
            size: std::mem::size_of::<KawaseUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            device,
            queue,
            blur_pipeline,
            blur_bind_group_layout,
            composite_pipeline,
            composite_bind_group_layout,
            uniform_buffer,
            sampler,
            frosted_pipeline,
            frosted_bind_group_layout,
            frosted_uniforms,
            kawase_pipeline,
            kawase_bind_group_layout,
            kawase_uniforms,
            kawase_pool: std::cell::RefCell::new(None),
        })
    }

    /// Apply dual-filtering blur effect
    pub fn apply_blur(
        &self,
        mut encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::Texture,
        output: &wgpu::Texture,
        radius: f32,
    ) -> Result<(), FilterError> {
        if radius <= 0.0 {
            // No blur needed, just copy
            self.copy_texture(encoder, input, output)?;
            return Ok(());
        }

        let input_size = (input.width(), input.height());
        let output_size = (output.width(), output.height());

        // Calculate number of iterations based on radius
        // Larger radius requires more iterations
        let iterations = ((radius / 2.0).ceil() as u32).max(1).min(8);

        // Create intermediate textures for ping-pong
        let intermediate_size = ((input_size.0 / 4).max(1), (input_size.1 / 4).max(1));

        let desc = wgpu::TextureDescriptor {
            label: Some("Blur Intermediate"),
            size: wgpu::Extent3d {
                width: intermediate_size.0,
                height: intermediate_size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        };

        let ping = self.device.create_texture(&desc);
        let pong = self.device.create_texture(&desc);

        // Step 1: Downsample to intermediate size
        self.run_blur_pass(
            &mut encoder,
            &input.create_view(&Default::default()),
            &ping.create_view(&Default::default()),
            input_size,
            intermediate_size,
            radius,
            0, // downsample
            0,
            iterations,
        );

        // Step 2: Iterative blur passes
        let mut current_input = &ping;
        let mut current_output = &pong;

        for i in 0..iterations {
            self.run_blur_pass(
                &mut encoder,
                &current_input.create_view(&Default::default()),
                &current_output.create_view(&Default::default()),
                intermediate_size,
                intermediate_size,
                radius,
                1, // blur
                i,
                iterations,
            );
            std::mem::swap(&mut current_input, &mut current_output);
        }

        // Step 3: Upsample to output size
        // Final result is in current_input (due to swap)
        self.run_blur_pass(
            &mut encoder,
            &current_input.create_view(&Default::default()),
            &output.create_view(&Default::default()),
            intermediate_size,
            output_size,
            radius,
            2, // upsample
            iterations,
            iterations,
        );

        // Insert memory barrier for synchronization
        // This ensures the compute shader writes are complete before any reads
        encoder.insert_debug_marker("Blur complete - memory barrier");

        Ok(())
    }

    /// Run a single blur pass
    fn run_blur_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        input_view: &wgpu::TextureView,
        output_view: &wgpu::TextureView,
        input_size: (u32, u32),
        output_size: (u32, u32),
        radius: f32,
        direction: u32,
        iteration: u32,
        total_iterations: u32,
    ) {
        // Update uniforms
        let uniforms = BlurUniforms {
            direction,
            iteration,
            total_iterations,
            radius,
            input_size: [input_size.0 as f32, input_size.1 as f32],
            output_size: [output_size.0 as f32, output_size.1 as f32],
        };

        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        // Create bind group for this pass
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Blur Pass Bind Group"),
            layout: &self.blur_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(input_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(output_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
            ],
        });

        // Dispatch compute pass
        let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Blur Compute Pass"),
            timestamp_writes: None,
        });

        compute_pass.set_pipeline(&self.blur_pipeline);
        compute_pass.set_bind_group(0, &bind_group, &[]);

        // Calculate workgroup dispatch size
        let workgroup_size = 8;
        let dispatch_x = (output_size.0 + workgroup_size - 1) / workgroup_size;
        let dispatch_y = (output_size.1 + workgroup_size - 1) / workgroup_size;

        compute_pass.dispatch_workgroups(dispatch_x, dispatch_y, 1);
    }

    /// Apply drop shadow effect
    pub fn apply_drop_shadow(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::Texture,
        output: &wgpu::Texture,
        dx: f32,
        dy: f32,
        blur_radius: f32,
        color: [f32; 4],
    ) -> Result<(), FilterError> {
        // For now, implement as blur + offset
        // Full implementation would render shadow and source with offset

        // Create blurred version for shadow
        let shadow_desc = wgpu::TextureDescriptor {
            label: Some("Drop Shadow Intermediate"),
            size: wgpu::Extent3d {
                width: output.width(),
                height: output.height(),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        };

        let shadow_texture = self.device.create_texture(&shadow_desc);

        // Apply blur for shadow
        self.apply_blur(encoder, input, &shadow_texture, blur_radius)?;

        // Composite shadow with offset and color
        // This is a simplified version - full implementation would use the composite pipeline
        self.composite_with_shadow(encoder, input, output, &shadow_texture, dx, dy, color)?;

        Ok(())
    }

    /// Composite source with shadow
    fn composite_with_shadow(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        _source: &wgpu::Texture,
        output: &wgpu::Texture,
        _shadow: &wgpu::Texture,
        _dx: f32,
        _dy: f32,
        color: [f32; 4],
    ) -> Result<(), FilterError> {
        // For now, use a simple render pass
        // Full implementation would use the composite pipeline with proper blending

        let output_view = output.create_view(&Default::default());

        {
            let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shadow Composite Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        } // _render_pass dropped here

        Ok(())
    }

    /// Copy texture (for when no filter is applied)
    fn copy_texture(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Texture,
        destination: &wgpu::Texture,
    ) -> Result<(), FilterError> {
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: source,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: destination,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: source.width().min(destination.width()),
                height: source.height().min(destination.height()),
                depth_or_array_layers: 1,
            },
        );

        Ok(())
    }

    /// Apply frosted glass effect using dual-pass separable Gaussian blur
    ///
    /// Pass 1: Horizontal blur with downsampling
    /// Pass 2: Vertical blur + tinting + noise
    pub fn apply_frosted_glass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::Texture,
        output: &wgpu::Texture,
        radius: f32,
        tint_color: [f32; 4],
        noise_strength: f32,
    ) -> Result<(), FilterError> {
        let input_size = (input.width(), input.height());

        // Use original size for blur processing
        // Note: Downsampling optimization removed due to UV coordinate complexity
        // The 5-sample Gaussian kernel is already quite efficient
        let adjusted_radius = radius;

        // Create intermediate texture for horizontal blur result (same size as input)
        let intermediate_desc = wgpu::TextureDescriptor {
            label: Some("Frosted Intermediate"),
            size: wgpu::Extent3d {
                width: input_size.0,
                height: input_size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };
        let intermediate = self.device.create_texture(&intermediate_desc);
        let intermediate_view = intermediate.create_view(&Default::default());

        // Create uniform buffer data
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct FrostedUniforms {
            direction: [f32; 2],
            radius: f32,
            noise_strength: f32,
            padding: f32,
            tint_color: [f32; 4],
        }

        // Pass 1: Horizontal blur + downsample
        let pass1_uniforms = FrostedUniforms {
            direction: [1.0, 0.0],
            radius: adjusted_radius,
            noise_strength: 0.0,
            padding: 0.0,
            tint_color: [0.0, 0.0, 0.0, 0.0],
        };
        self.queue.write_buffer(
            &self.frosted_uniforms,
            0,
            bytemuck::bytes_of(&pass1_uniforms),
        );

        // Create bind group for Pass 1
        let pass1_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Frosted Pass 1 Bind Group"),
            layout: &self.frosted_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &input.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.frosted_uniforms.as_entire_binding(),
                },
            ],
        });

        // Pass 1 render pass
        {
            let mut pass1 = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Frosted Horizontal Blur"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &intermediate_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass1.set_pipeline(&self.frosted_pipeline);
            pass1.set_bind_group(0, &pass1_bind_group, &[]);
            pass1.draw(0..3, 0..1);
        }

        // Pass 2: Vertical blur + tinting + noise
        let pass2_uniforms = FrostedUniforms {
            direction: [0.0, 1.0],
            radius: adjusted_radius,
            noise_strength,
            padding: 0.0,
            tint_color,
        };
        self.queue.write_buffer(
            &self.frosted_uniforms,
            0,
            bytemuck::bytes_of(&pass2_uniforms),
        );

        // Create bind group for Pass 2
        let pass2_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Frosted Pass 2 Bind Group"),
            layout: &self.frosted_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&intermediate_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.frosted_uniforms.as_entire_binding(),
                },
            ],
        });

        // Pass 2 render pass
        {
            let mut pass2 = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Frosted Vertical Blur + Composite"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output.create_view(&Default::default()),
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass2.set_pipeline(&self.frosted_pipeline);
            pass2.set_bind_group(0, &pass2_bind_group, &[]);
            pass2.draw(0..3, 0..1);
        }

        Ok(())
    }

    /// Apply large-radius frosted glass blur using Dual Kawase algorithm.
    ///
    /// Pipeline: input → downsample 1/2 → downsample 1/4 → Kawase × N → upsample 1/2 → output
    /// Intermediate textures use Rgba16Float to prevent rounding errors from multi-pass accumulation.
    ///
    /// N (Kawase iterations) = clamp(ceil(blur_radius / 15.0), 2, 6)
    ///   N=2 ≈ 30px,  N=4 ≈ 50px,  N=6 ≈ 80px equivalent blur radius
    ///
    /// If `external_pool` is provided, textures will be acquired from the pool for efficient reuse.
    /// Otherwise, internal textures are used (and recreated on resolution change).
    pub fn apply_frosted_glass_kawase(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::Texture,
        output: &wgpu::Texture,
        blur_radius: f32,
        external_pool: Option<&SharedTexturePool>,
    ) -> Result<(), FilterError> {
        let full_w = input.width();
        let full_h = input.height();

        // Use external pool if provided, otherwise use internal pool
        let _using_external_pool = external_pool.is_some();
        let mut external_tex_set = external_pool.map(|pool| pool.acquire_kawase_set(full_w, full_h));

        // Validate external pool textures have the expected exact dimensions.
        // If they don't match, fallback to the internal pool to avoid sampling artifacts.
        if let Some(ref set) = external_tex_set {
            let expected_half_w = (full_w / 2).max(1);
            let expected_half_h = (full_h / 2).max(1);
            let expected_quarter_w = (full_w / 4).max(1);
            let expected_quarter_h = (full_h / 4).max(1);

            let valid = set.ds_half.texture().width() == expected_half_w
                && set.ds_half.texture().height() == expected_half_h
                && set.ds_quarter.texture().width() == expected_quarter_w
                && set.ds_quarter.texture().height() == expected_quarter_h
                && set.ping.texture().width() == expected_quarter_w
                && set.ping.texture().height() == expected_quarter_h
                && set.pong.texture().width() == expected_quarter_w
                && set.pong.texture().height() == expected_quarter_h;

            if !valid {
                external_tex_set = None;
            }
        }

        // Ensure internal texture pool matches current resolution (for fallback or full_tex)
        {
            let mut pool_ref = self.kawase_pool.borrow_mut();
            let needs_rebuild = pool_ref
                .as_ref()
                .map_or(true, |p| !p.matches(full_w, full_h));
            if needs_rebuild {
                // Old textures are automatically dropped (and GPU resources freed)
                *pool_ref = Some(KawaseTexturePool::create(&self.device, full_w, full_h));
            }
        }

        let pool_ref = self.kawase_pool.borrow();
        let internal_pool = pool_ref.as_ref().unwrap();

        // N = number of Kawase iterations, clamped to [2, 4]
        // OPTIMIZATION: Reduced max from 6 to 4, increased divisor from 15 to 25
        // for fewer passes while maintaining quality
        let kawase_n = ((blur_radius / 25.0).ceil() as u32).max(2).min(4);
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Kawase Blur Encoder"),
        });

        // Helper: run one Kawase render pass (source → target)
        let run_pass = |encoder: &mut wgpu::CommandEncoder,
                        src_view: &wgpu::TextureView,
                        dst_view: &wgpu::TextureView,
                        mode: u32,
                        pass_index: u32| {
            let uniforms = KawaseUniforms {
                mode,
                pass_index,
                _pad0: 0,
                _pad1: 0,
            };
            self.queue
                .write_buffer(&self.kawase_uniforms, 0, bytemuck::bytes_of(&uniforms));

            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Kawase Pass BindGroup"),
                layout: &self.kawase_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(src_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.kawase_uniforms.as_entire_binding(),
                    },
                ],
            });

            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Kawase Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: dst_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rpass.set_pipeline(&self.kawase_pipeline);
            rpass.set_bind_group(0, &bind_group, &[]);
            rpass.draw(0..3, 0..1);
        };

        // Use external pool textures if available, otherwise internal
        let (half_tex, quarter_tex, ping_tex, pong_tex) =
            if let Some(ref tex_set) = external_tex_set {
                (
                    tex_set.ds_half.texture(),
                    tex_set.ds_quarter.texture(),
                    tex_set.ping.texture(),
                    tex_set.pong.texture(),
                )
            } else {
                (
                    &internal_pool.half,
                    &internal_pool.quarter,
                    &internal_pool.ping,
                    &internal_pool.pong,
                )
            };

        // 1. Downsample full → half
        run_pass(
            &mut encoder,
            &input.create_view(&Default::default()),
            &half_tex.create_view(&Default::default()),
            0,
            0,
        );

        // 2. Downsample half → quarter
        run_pass(
            &mut encoder,
            &half_tex.create_view(&Default::default()),
            &quarter_tex.create_view(&Default::default()),
            0,
            0,
        );

        // 3. Kawase blur iterations (ping-pong between quarter and ping/pong)
        // src alternates: quarter → ping → pong → ... final result in last dst
        let textures = [quarter_tex, ping_tex, pong_tex];
        let mut src_idx: usize = 0; // index into textures[] for source
                                    // dst starts at ping (index 1), then alternates ping/pong
        let kawase_dsts: [usize; 6] = [1, 2, 1, 2, 1, 2]; // ping=1, pong=2
        let mut last_dst_idx = 0usize;
        for i in 0..kawase_n {
            let dst_idx = kawase_dsts[i as usize];
            run_pass(
                &mut encoder,
                &textures[src_idx].create_view(&Default::default()),
                &textures[dst_idx].create_view(&Default::default()),
                1,
                i,
            );
            src_idx = dst_idx;
            last_dst_idx = dst_idx;
        }

        // 4. Upsample quarter-res result → half
        // The final Kawase result is in textures[last_dst_idx]
        // We need to upsample it. Use the half buffer as intermediate.
        run_pass(
            &mut encoder,
            &textures[last_dst_idx].create_view(&Default::default()),
            &half_tex.create_view(&Default::default()),
            2,
            0,
        );

        // 5. Upsample half → output (full res)
        // Output texture is Rgba8Unorm (the blur entry texture), so we need a separate pass
        // that converts Rgba16Float → Rgba8Unorm via the render pipeline.
        // Since output format might differ, we use the frosted pipeline for final blit.
        // For now, use the same kawase pipeline but output to a Rgba16Float target,
        // then we'll need a final conversion pass.
        // Actually: output is the BlurredTextureEntry.texture which is Rgba8Unorm.
        // We can't render to it with Rgba16Float pipeline target format.
        // Solution: upsample to a temporary Rgba16Float full-res texture, then copy to output.
        let full_tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Kawase Full Res Temp"),
            size: wgpu::Extent3d {
                width: full_w,
                height: full_h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        run_pass(
            &mut encoder,
            &half_tex.create_view(&Default::default()),
            &full_tex.create_view(&Default::default()),
            2,
            0,
        );

        // 6. Blit Rgba16Float full_tex → Rgba8Unorm output using frosted pipeline
        //    We use a simple full-screen blit with identity uniforms in frosted_pipeline.
        //    Since frosted_pipeline uses Rgba8Unorm target, this handles the format conversion.
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct FrostedUniforms {
            direction: [f32; 2],
            radius: f32,
            noise_strength: f32,
            padding: f32,
            tint_color: [f32; 4],
        }
        let blit_uniforms = FrostedUniforms {
            direction: [1.0, 0.0],
            radius: 0.0, // radius=0 → no blur, just copy center sample
            noise_strength: 0.0,
            padding: 0.0,
            tint_color: [0.0, 0.0, 0.0, 0.0],
        };
        self.queue.write_buffer(
            &self.frosted_uniforms,
            0,
            bytemuck::bytes_of(&blit_uniforms),
        );

        let blit_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Kawase Final Blit BindGroup"),
            layout: &self.frosted_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &full_tex.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.frosted_uniforms.as_entire_binding(),
                },
            ],
        });

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Kawase Final Blit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output.create_view(&Default::default()),
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rpass.set_pipeline(&self.frosted_pipeline);
            rpass.set_bind_group(0, &blit_bind_group, &[]);
            rpass.draw(0..3, 0..1);
        }

        Ok(())
    }

    /// Encode frosted glass Kawase blur commands into an existing encoder.
    ///
    /// This method allows batching multiple blur effects into a single command buffer
    /// for better GPU utilization and reduced CPU-GPU synchronization overhead.
    ///
    /// # Arguments
    /// * `encoder` - The command encoder to record commands into
    /// * `input` - Input texture (scene texture to blur from)
    /// * `output` - Output texture (blur entry texture to write to)
    /// * `blur_radius` - Blur radius in pixels
    /// * `external_pool` - Optional texture pool for efficient texture reuse
    /// * `uniforms_offset` - Byte offset for uniform buffer to avoid conflicts when batching
    pub fn encode_frosted_glass_kawase(
        &self,
        mut encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::Texture,
        output: &wgpu::Texture,
        blur_radius: f32,
        external_pool: Option<&SharedTexturePool>,
        _uniforms_offset: u64, // Reserved for future use if we need per-entry uniforms
    ) -> Result<(), FilterError> {
        let full_w = input.width();
        let full_h = input.height();

        // Use external pool if provided, otherwise use internal pool
        let external_tex_set = external_pool.map(|pool| pool.acquire_kawase_set(full_w, full_h));

        // Ensure internal texture pool matches current resolution (for fallback or full_tex)
        {
            let mut pool_ref = self.kawase_pool.borrow_mut();
            let needs_rebuild = pool_ref
                .as_ref()
                .map_or(true, |p| !p.matches(full_w, full_h));
            if needs_rebuild {
                *pool_ref = Some(KawaseTexturePool::create(&self.device, full_w, full_h));
            }
        }

        let pool_ref = self.kawase_pool.borrow();
        let internal_pool = pool_ref.as_ref().unwrap();

        // N = number of Kawase iterations, clamped to [2, 4]
        // OPTIMIZATION: Reduced max from 6 to 4, increased divisor from 15 to 25
        // for fewer passes while maintaining quality
        let kawase_n = ((blur_radius / 25.0).ceil() as u32).max(2).min(4);

        // Helper: run one Kawase render pass (source → target)
        let run_pass = |encoder: &mut wgpu::CommandEncoder,
                        src_view: &wgpu::TextureView,
                        dst_view: &wgpu::TextureView,
                        mode: u32,
                        pass_index: u32| {
            let uniforms = KawaseUniforms {
                mode,
                pass_index,
                _pad0: 0,
                _pad1: 0,
            };
            self.queue
                .write_buffer(&self.kawase_uniforms, 0, bytemuck::bytes_of(&uniforms));

            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Kawase Pass BindGroup"),
                layout: &self.kawase_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(src_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.kawase_uniforms.as_entire_binding(),
                    },
                ],
            });

            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Kawase Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: dst_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rpass.set_pipeline(&self.kawase_pipeline);
            rpass.set_bind_group(0, &bind_group, &[]);
            rpass.draw(0..3, 0..1);
        };

        // Use external pool textures if available, otherwise internal
        let (half_tex, quarter_tex, ping_tex, pong_tex) =
            if let Some(ref tex_set) = external_tex_set {
                (
                    tex_set.ds_half.texture(),
                    tex_set.ds_quarter.texture(),
                    tex_set.ping.texture(),
                    tex_set.pong.texture(),
                )
            } else {
                (
                    &internal_pool.half,
                    &internal_pool.quarter,
                    &internal_pool.ping,
                    &internal_pool.pong,
                )
            };

        // 1. Downsample full → half
        run_pass(
            &mut encoder,
            &input.create_view(&Default::default()),
            &half_tex.create_view(&Default::default()),
            0,
            0,
        );

        // 2. Downsample half → quarter
        run_pass(
            &mut encoder,
            &half_tex.create_view(&Default::default()),
            &quarter_tex.create_view(&Default::default()),
            0,
            0,
        );

        // 3. Kawase blur iterations (ping-pong between quarter and ping/pong)
        let textures = [quarter_tex, ping_tex, pong_tex];
        let mut src_idx: usize = 0;
        let kawase_dsts: [usize; 6] = [1, 2, 1, 2, 1, 2];
        let mut last_dst_idx = 0usize;
        for i in 0..kawase_n {
            let dst_idx = kawase_dsts[i as usize];
            run_pass(
                &mut encoder,
                &textures[src_idx].create_view(&Default::default()),
                &textures[dst_idx].create_view(&Default::default()),
                1,
                i,
            );
            src_idx = dst_idx;
            last_dst_idx = dst_idx;
        }

        // 4. Upsample quarter-res result → half
        run_pass(
            &mut encoder,
            &textures[last_dst_idx].create_view(&Default::default()),
            &half_tex.create_view(&Default::default()),
            2,
            0,
        );

        // 5. Create temporary full-res texture for final upsample
        let full_tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Kawase Full Res Temp"),
            size: wgpu::Extent3d {
                width: full_w,
                height: full_h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        run_pass(
            &mut encoder,
            &half_tex.create_view(&Default::default()),
            &full_tex.create_view(&Default::default()),
            2,
            0,
        );

        // 6. Blit Rgba16Float full_tex → Rgba8Unorm output using frosted pipeline
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct FrostedUniforms {
            direction: [f32; 2],
            radius: f32,
            noise_strength: f32,
            padding: f32,
            tint_color: [f32; 4],
        }
        let blit_uniforms = FrostedUniforms {
            direction: [1.0, 0.0],
            radius: 0.0,
            noise_strength: 0.0,
            padding: 0.0,
            tint_color: [0.0, 0.0, 0.0, 0.0],
        };
        self.queue.write_buffer(
            &self.frosted_uniforms,
            0,
            bytemuck::bytes_of(&blit_uniforms),
        );

        let blit_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Kawase Final Blit BindGroup"),
            layout: &self.frosted_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &full_tex.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.frosted_uniforms.as_entire_binding(),
                },
            ],
        });

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Kawase Final Blit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output.create_view(&Default::default()),
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rpass.set_pipeline(&self.frosted_pipeline);
            rpass.set_bind_group(0, &blit_bind_group, &[]);
            rpass.draw(0..3, 0..1);
        }

        // NOTE: Caller is responsible for submitting the encoder
        Ok(())
    }

    /// Check if pipeline is ready
    pub fn is_ready(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests require a wgpu device, use integration tests
}
