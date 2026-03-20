use vello::{Scene, peniko::{Color, Fill}};
use kurbo::{Affine, Rect as KRect, RoundedRect, Vec2};
use taffy::prelude::*;
use crate::state::SharedState;
use crate::platform::SurfaceState;
use crate::engine::EngineState;

pub fn render_node_recursive(id: u32, state: &SharedState, scene: &mut Scene, parent_pos: Vec2) {
    if let Some(node) = state.nodes.get(&id) {
        let layout = state.taffy.layout(node.taffy_node).unwrap();
        let global_pos = parent_pos + Vec2::new(layout.location.x as f64, layout.location.y as f64);
        let rect = KRect::from_origin_size((global_pos.x, global_pos.y), (layout.size.width as f64, layout.size.height as f64));
        if node.border_radius > 0.0 {
            let rounded = RoundedRect::from_rect(rect, node.border_radius as f64);
            scene.fill(Fill::NonZero, Affine::IDENTITY, node.color, None, &rounded);
        } else {
            scene.fill(Fill::NonZero, Affine::IDENTITY, node.color, None, &rect);
        }
        for &child_id in &node.children { 
            render_node_recursive(child_id, state, scene, global_pos); 
        }
    }
}

pub fn render_frame(e: &mut EngineState, s: &mut SurfaceState) {
    let w = s.surface.config.width; 
    let h = s.surface.config.height; 
    if w == 0 || h == 0 { return; }
    
    let rid = { 
        let mut g = e.shared_state.lock().unwrap(); 
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
        let g = e.shared_state.lock().unwrap(); 
        render_node_recursive(id, &g, &mut scene, Vec2::ZERO); 
    }
    
    if s.offscreen_texture.as_ref().map_or(true, |(t, _)| t.width() != w || t.height() != h) {
        let texture = e.context.devices[s.surface.dev_id].device.create_texture(&vello::wgpu::TextureDescriptor { 
            label: None, 
            size: vello::wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, 
            mip_level_count: 1, 
            sample_count: 1, 
            dimension: vello::wgpu::TextureDimension::D2, 
            format: vello::wgpu::TextureFormat::Rgba8Unorm, 
            usage: vello::wgpu::TextureUsages::STORAGE_BINDING | vello::wgpu::TextureUsages::TEXTURE_BINDING, 
            view_formats: &[] 
        });
        let bg = e.context.devices[s.surface.dev_id].device.create_bind_group(&vello::wgpu::BindGroupDescriptor { 
            label: None, 
            layout: &e.blit_bind_group_layout, 
            entries: &[
                vello::wgpu::BindGroupEntry { 
                    binding: 0, 
                    resource: vello::wgpu::BindingResource::TextureView(&texture.create_view(&Default::default())) 
                }, 
                vello::wgpu::BindGroupEntry { 
                    binding: 1, 
                    resource: vello::wgpu::BindingResource::Sampler(&e.sampler) 
                }
            ] 
        });
        s.offscreen_texture = Some((texture, bg));
    }
    
    let (off_t, blit_bg) = s.offscreen_texture.as_ref().unwrap();
    e.renderer.render_to_texture(
        &e.context.devices[s.surface.dev_id].device, 
        &e.context.devices[s.surface.dev_id].queue, 
        &scene, 
        &off_t.create_view(&Default::default()), 
        &vello::RenderParams { 
            base_color: Color::BLACK, 
            width: w, 
            height: h, 
            antialiasing_method: vello::AaConfig::Area 
        }
    ).unwrap();
    
    if let Ok(st) = s.surface.surface.get_current_texture() {
        let mut enc = e.context.devices[s.surface.dev_id].device.create_command_encoder(&Default::default());
        { 
            let mut rp = enc.begin_render_pass(&vello::wgpu::RenderPassDescriptor { 
                label: None, 
                color_attachments: &[Some(vello::wgpu::RenderPassColorAttachment { 
                    view: &st.texture.create_view(&Default::default()), 
                    resolve_target: None, 
                    ops: vello::wgpu::Operations { 
                        load: vello::wgpu::LoadOp::Clear(vello::wgpu::Color::TRANSPARENT), 
                        store: vello::wgpu::StoreOp::Store 
                    }, 
                    depth_slice: None 
                })], 
                depth_stencil_attachment: None, 
                timestamp_writes: None, 
                occlusion_query_set: None 
            }); 
            rp.set_pipeline(&s.blit_pipeline); 
            rp.set_bind_group(0, blit_bg, &[]); 
            rp.draw(0..3, 0..1); 
        }
        e.context.devices[s.surface.dev_id].queue.submit(Some(enc.finish())); 
        st.present();
    }
}
