// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::sync::atomic::AtomicBool;
use vello::{Renderer, RendererOptions, Scene, peniko::{Color, Fill}};
use dyxel_render_api::{RenderBackend, SurfaceState, LifecycleEvent, RenderContext as ApiRenderContext, SharedPtr, SharedMutex};
use dyxel_render_api::types::{BackendConfig, RenderResult};
#[cfg(target_arch = "wasm32")]
use dyxel_render_api::LockExt;
use vello::wgpu;
use kurbo::{Affine, Rect as KRect, RoundedRect, Vec2};
use taffy::style::AvailableSpace;
use dyxel_shared::SharedState;

#[cfg(target_os = "macos")]
pub mod mac;
#[cfg(target_os = "android")]
pub mod android;
#[cfg(target_arch = "wasm32")]
pub mod web;

pub struct VelloBackend {
    pub renderer: SharedMutex<Option<Renderer>>,
    pub blit_bind_group_layout: SharedMutex<Option<wgpu::BindGroupLayout>>,
    pub sampler: SharedMutex<Option<wgpu::Sampler>>,
    pub blit_shader: SharedMutex<Option<wgpu::ShaderModule>>,
    pub blit_pipeline: SharedMutex<Option<wgpu::RenderPipeline>>,
    pub pipeline_cache: SharedMutex<Option<wgpu::PipelineCache>>,
    pub cache_path: SharedMutex<Option<String>>,
    pub cache_saved: AtomicBool,
}

const BLIT_SHADER_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/blit.spv"));

impl VelloBackend {
    pub fn new() -> Self {
        Self {
            renderer: SharedMutex::new(None),
            blit_bind_group_layout: SharedMutex::new(None),
            sampler: SharedMutex::new(None),
            blit_shader: SharedMutex::new(None),
            blit_pipeline: SharedMutex::new(None),
            pipeline_cache: SharedMutex::new(None),
            cache_path: SharedMutex::new(None),
            cache_saved: AtomicBool::new(false),
        }
    }

    fn save_cache(&self) {
        if self.cache_saved.load(std::sync::atomic::Ordering::SeqCst) { return; }
        let cache_lock = self.pipeline_cache.lock().unwrap();
        let path_lock = self.cache_path.lock().unwrap();
        if let (Some(cache), Some(path)) = (&*cache_lock, &*path_lock) {
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(data) = cache.get_data() {
                if let Err(e) = std::fs::write(path, &data) {
                    log::error!("VelloBackend: Failed to save pipeline cache: {}", e);
                } else {
                    log::info!("VelloBackend: Pipeline cache saved to {} ({} bytes)", path, data.len());
                    self.cache_saved.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }
            #[cfg(target_arch = "wasm32")]
            let _ = (cache, path);
        }
    }

    /// Prewarm pipelines: create all necessary pipelines in background to reduce first-render latency
    fn prewarm_pipelines(&self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        log::info!("VelloBackend: Prewarming pipelines...");
        let blit_shader = self.blit_shader.lock().unwrap();
        let blit_layout = self.blit_bind_group_layout.lock().unwrap();
        
        if let (Some(shader), Some(layout)) = (&*blit_shader, &*blit_layout) {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Blit Pipeline Layout Prewarm"),
                bind_group_layouts: &[layout],
                push_constant_ranges: &[]
            });

            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Blit Pipeline Prewarm"),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL
                    })],
                    compilation_options: Default::default()
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: self.pipeline_cache.lock().unwrap().as_ref()
            });
            *self.blit_pipeline.lock().unwrap() = Some(pipeline);
        }
        log::info!("VelloBackend: Pipeline prewarming complete.");
    }

    fn render_internal(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        v_surface_surface: &mut vello::util::RenderSurface<'static>,
        blit_pipeline: &wgpu::RenderPipeline,
        offscreen_texture: &mut Option<(wgpu::Texture, wgpu::TextureView, wgpu::BindGroup)>,
        shared_state: &SharedMutex<SharedState>,
    ) -> RenderResult {
        let mut renderer_lock = self.renderer.lock().unwrap();
        let renderer = renderer_lock.as_mut().ok_or_else(|| anyhow::anyhow!("Renderer not initialized"))?;

        let w = v_surface_surface.config.width;
        let h = v_surface_surface.config.height;
        if w == 0 || h == 0 { return Ok(()); }

        let rid = { 
            let mut g = shared_state.lock().unwrap(); 
            g.root_id.map(|id| { 
                if let Some(rn) = g.nodes.get(&id).map(|n| n.taffy_node) { 
                    let _ = g.taffy.compute_layout(rn, taffy::prelude::Size { 
                        width: AvailableSpace::Definite(w as f32), 
                        height: AvailableSpace::Definite(h as f32) 
                    }); 
                } 
                id 
            }) 
        };
        
        let mut scene = Scene::new();

        if let Some(id) = rid { 
            let g = shared_state.lock().unwrap(); 
            render_node_recursive_with_transform(id, &g, &mut scene, Vec2::ZERO, Affine::IDENTITY); 
        }

        // Offscreen logic alignment
        if offscreen_texture.as_ref().map_or(true, |(t, _, _)| t.width() != w || t.height() != h) {
            let texture = device.create_texture(&wgpu::TextureDescriptor { 
                label: Some("Vello Offscreen Texture"), 
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, 
                mip_level_count: 1, 
                sample_count: 1, 
                dimension: wgpu::TextureDimension::D2, 
                format: wgpu::TextureFormat::Rgba8Unorm, 
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING, 
                view_formats: &[] 
            });
            let view = texture.create_view(&Default::default());
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor { 
                label: Some("Vello Blit Bind Group"), 
                layout: self.blit_bind_group_layout.lock().unwrap().as_ref().unwrap(), 
                entries: &[
                    wgpu::BindGroupEntry { 
                        binding: 0, 
                        resource: wgpu::BindingResource::TextureView(&view) 
                    }, 
                    wgpu::BindGroupEntry { 
                        binding: 1, 
                        resource: wgpu::BindingResource::Sampler(self.sampler.lock().unwrap().as_ref().unwrap()) 
                    }
                ] 
            });
            *offscreen_texture = Some((texture, view, bg));
        }
        
        let (_, off_view, blit_bg) = offscreen_texture.as_ref().unwrap();
        
        renderer.render_to_texture(
            device,
            queue,
            &scene,
            off_view,
            &vello::RenderParams {
                base_color: Color::TRANSPARENT,
                width: w,
                height: h,
                antialiasing_method: vello::AaConfig::Area
            }
        ).map_err(|e| anyhow::anyhow!("Vello render error: {:?}", e))?;
        
        if let Ok(st) = v_surface_surface.surface.get_current_texture() {
            let mut enc = device.create_command_encoder(&Default::default());
            { 
                let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor { 
                    label: Some("Vello Blit Pass"), 
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { 
                        view: &st.texture.create_view(&Default::default()), 
                        resolve_target: None, 
                        ops: wgpu::Operations { 
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), 
                            store: wgpu::StoreOp::Store 
                        }, 
                        depth_slice: None 
                    })], 
                    depth_stencil_attachment: None, 
                    timestamp_writes: None, 
                    occlusion_query_set: None 
                }); 
                rp.set_pipeline(blit_pipeline); 
                rp.set_bind_group(0, blit_bg, &[]); 
                rp.draw(0..3, 0..1); 
            }
            queue.submit(Some(enc.finish())); 
            st.present();
        }

        Ok(())
    }
}

fn render_node_recursive_with_transform(id: u32, state: &SharedState, scene: &mut Scene, parent_pos: Vec2, transform: Affine) {
    if let Some(node) = state.nodes.get(&id) {
        let layout = state.taffy.layout(node.taffy_node).unwrap();
        let global_pos = parent_pos + Vec2::new(layout.location.x as f64, layout.location.y as f64);
        let rect = KRect::from_origin_size((global_pos.x, global_pos.y), (layout.size.width as f64, layout.size.height as f64));
        if node.border_radius > 0.0 {
            let rounded = RoundedRect::from_rect(rect, node.border_radius as f64);
            scene.fill(Fill::NonZero, transform, node.color, None, &rounded);
        } else {
            scene.fill(Fill::NonZero, transform, node.color, None, &rect);
        }
        for &child_id in &node.children { 
            render_node_recursive_with_transform(child_id, state, scene, global_pos, transform); 
        }
    }
}

impl RenderBackend for VelloBackend {
    fn init(&self, device: &wgpu::Device, queue: &wgpu::Queue, config: BackendConfig) -> anyhow::Result<()> {
        log::info!("VelloBackend: Initializing...");
        
        // Try using pre-compiled SPIR-V, fall back to WGSL if it fails
        let blit_shader = if cfg!(target_os = "android") {
            log::info!("VelloBackend: Attempting to use SPIR-V for blit shader on Android");
            // Ensure SPIR-V data is properly aligned
            let spv_words: Vec<u32> = BLIT_SHADER_SPV
                .chunks_exact(4)
                .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect();
            
            log::info!("VelloBackend: SPIR-V data size: {} bytes, {} words", BLIT_SHADER_SPV.len(), spv_words.len());
            
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Blit Shader (SPIR-V)"),
                source: wgpu::ShaderSource::SpirV(std::borrow::Cow::Owned(spv_words)),
            })
        } else {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Blit Shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into())
            })
        };

        let blit_bl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false
                    },
                    count: None
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None
                }
            ]
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let cache_path = format!("{}/vello_v1.cache", config.data_dir);
        
        #[cfg(not(target_arch = "wasm32"))]
        let cache_data = std::fs::read(&cache_path).ok();
        #[cfg(target_arch = "wasm32")]
        let cache_data: Option<Vec<u8>> = None;
        
        let pipeline_cache = if device.features().contains(wgpu::Features::PIPELINE_CACHE) {
            Some(unsafe {
                device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
                    label: Some("Vello Pipeline Cache"),
                    data: cache_data.as_deref(),
                    fallback: true,
                })
            })
        } else {
            None
        };

                #[cfg(not(target_arch = "wasm32"))]
        let num_threads = std::thread::available_parallelism().ok().map(|n| n.get().max(8));
        #[cfg(target_arch = "wasm32")]
        let num_threads = None;

        let renderer = Renderer::new(device, RendererOptions {
            antialiasing_support: vello::AaSupport::area_only(),
            pipeline_cache: pipeline_cache.clone(),
            num_init_threads: num_threads.and_then(|n| std::num::NonZeroUsize::new(n)),
            use_cpu: false
        }).map_err(|e| anyhow::anyhow!("Failed to create renderer: {}", e))?;

        *self.renderer.lock().unwrap() = Some(renderer);
        *self.blit_bind_group_layout.lock().unwrap() = Some(blit_bl);
        *self.sampler.lock().unwrap() = Some(sampler);
        *self.blit_shader.lock().unwrap() = Some(blit_shader);
        *self.pipeline_cache.lock().unwrap() = pipeline_cache;
        *self.cache_path.lock().unwrap() = Some(cache_path);

        // Prewarm pipelines
        self.prewarm_pipelines(device, wgpu::TextureFormat::Rgba8Unorm);

        // Warmup draw
        {
            let mut renderer_lock = self.renderer.lock().unwrap();
            let renderer = renderer_lock.as_mut().unwrap();
            let dummy_texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Warmup Dummy Texture"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::STORAGE_BINDING,
                view_formats: &[],
            });
            let dummy_view = dummy_texture.create_view(&wgpu::TextureViewDescriptor::default());
            let scene = Scene::new();
            let params = vello::RenderParams {
                base_color: Color::TRANSPARENT,
                width: 1,
                height: 1,
                antialiasing_method: vello::AaConfig::Area,
            };
            let _ = renderer.render_to_texture(device, queue, &scene, &dummy_view, &params);
            log::info!("VelloBackend: Warmup draw complete.");
        }

        Ok(())
    }

    fn create_surface_state(
        &self,
        context: &mut ApiRenderContext,
        target: Option<wgpu::SurfaceTarget<'static>>,
        _surface_ptr: u64,
        width: u32,
        height: u32,
    ) -> anyhow::Result<Box<dyn SurfaceState>> {
        let surface = pollster::block_on(context.create_surface(target.expect("Vello requires a surface target"), width, height, wgpu::PresentMode::AutoVsync))
            .map_err(|e| anyhow::anyhow!("Failed to create surface: {:?}", e))?;
        
        let blit_layout_lock = self.blit_bind_group_layout.lock().unwrap();
        let blit_shader_lock = self.blit_shader.lock().unwrap();
        
        let device = &context.devices[surface.dev_id].device;

        let bl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[blit_layout_lock.as_ref().unwrap()],
            push_constant_ranges: &[]
        });

        let blit_p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&bl),
            vertex: wgpu::VertexState {
                module: blit_shader_lock.as_ref().unwrap(),
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: blit_shader_lock.as_ref().unwrap(),
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface.config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL
                })],
                compilation_options: Default::default()
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: self.pipeline_cache.lock().unwrap().as_ref()
        });

        #[cfg(target_os = "macos")]
        return Ok(Box::new(mac::MacVelloSurfaceState {
            surface,
            blit_pipeline: blit_p,
            offscreen_texture: None,
        }));

        #[cfg(target_os = "android")]
        return Ok(Box::new(android::AndroidVelloSurfaceState {
            surface,
            blit_pipeline: blit_p,
            offscreen_texture: None,
        }));

        #[cfg(target_arch = "wasm32")]
        return Ok(Box::new(web::WebVelloSurfaceState {
            surface,
            blit_pipeline: blit_p,
            offscreen_texture: None,
        }));

        #[cfg(all(not(target_os = "macos"), not(target_os = "android"), not(target_arch = "wasm32")))]
        Err(anyhow::anyhow!("Unsupported platform"))
    }

    fn prepare(&self, _shared_state: &SharedPtr<SharedMutex<SharedState>>, _width: u32, _height: u32) {}

    fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface: &mut dyn SurfaceState,
        shared_state: &SharedPtr<SharedMutex<SharedState>>,
    ) -> RenderResult {
        #[cfg(target_os = "macos")]
        {
            let v_surface = surface.as_any_mut().downcast_mut::<mac::MacVelloSurfaceState>().ok_or_else(|| anyhow::anyhow!("Invalid surface state (not MacVelloSurfaceState)"))?;
            return self.render_internal(device, queue, &mut v_surface.surface, &v_surface.blit_pipeline, &mut v_surface.offscreen_texture, shared_state);
        }

        #[cfg(target_os = "android")]
        {
            let v_surface = surface.as_any_mut().downcast_mut::<android::AndroidVelloSurfaceState>().ok_or_else(|| anyhow::anyhow!("Invalid surface state (not AndroidVelloSurfaceState)"))?;
            return self.render_internal(device, queue, &mut v_surface.surface, &v_surface.blit_pipeline, &mut v_surface.offscreen_texture, shared_state);
        }

        #[cfg(target_arch = "wasm32")]
        {
            let v_surface = surface.as_any_mut().downcast_mut::<web::WebVelloSurfaceState>().ok_or_else(|| anyhow::anyhow!("Invalid surface state (not WebVelloSurfaceState)"))?;
            return self.render_internal(device, queue, &mut v_surface.surface, &v_surface.blit_pipeline, &mut v_surface.offscreen_texture, shared_state);
        }

        #[cfg(all(not(target_os = "macos"), not(target_os = "android"), not(target_arch = "wasm32")))]
        Err(anyhow::anyhow!("Unsupported platform"))
    }

    fn on_lifecycle_event(&self, event: LifecycleEvent) {
        match event {
            LifecycleEvent::FirstFrameDone | LifecycleEvent::Shutdown => {
                self.save_cache();
            }
            _ => {}
        }
    }
}
