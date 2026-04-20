// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::engine::RenderState;
use dyxel_render_api::{
    BlurEffect, DeviceHandle, NodeContent, PreparedText, QueueHandle, RenderPackage, SceneNode,
    ShadowDesc, SurfaceState, TextDecoration, TextDrawPayload, TextGlyph, TextGlyphRun, Transform,
};
use dyxel_shared::ViewType;
use std::collections::{HashMap, HashSet};
use taffy::style::AvailableSpace;

fn build_scene_snapshot(
    state: &dyxel_shared::SharedState,
    editors: &mut std::collections::HashMap<u32, dyxel_editor::Editor>,
) -> Vec<SceneNode> {
    fn recurse(
        state: &dyxel_shared::SharedState,
        editors: &mut std::collections::HashMap<u32, dyxel_editor::Editor>,
        id: u32,
        parent_pos: (f32, f32),
        out: &mut Vec<SceneNode>,
    ) {
        let Some(node) = state.nodes.get(&id) else { return };
        let layout = state.taffy.layout(node.taffy_node).unwrap();
        let x = layout.location.x;
        let y = layout.location.y;
        let width = layout.size.width;
        let height = layout.size.height;
        let abs_x = parent_pos.0 + x;
        let abs_y = parent_pos.1 + y;

        let content = if node.view_type == dyxel_shared::ViewType::Text {
            let prepared = if let Some(editor) = editors.get_mut(&id) {
                let mut glyph_runs = Vec::new();
                let mut decorations = Vec::new();
                editor.draw_with_callback(|cmd| match cmd {
                    dyxel_editor::DrawCommand::FillRect { x, y, width, height, color } => {
                        decorations.push(TextDecoration {
                            x,
                            y,
                            width,
                            height,
                            color: peniko::Color::from_rgba8(color[0], color[1], color[2], color[3]),
                        });
                    }
                    dyxel_editor::DrawCommand::DrawGlyphs { font_data, font_size, color, glyphs } => {
                        let glyph_vec = glyphs
                            .into_iter()
                            .map(|g| TextGlyph { id: g.id, x: g.x, y: g.y })
                            .collect();
                        glyph_runs.push(TextGlyphRun {
                            font_data,
                            font_size,
                            color: peniko::Color::from_rgba8(color[0], color[1], color[2], color[3]),
                            glyphs: glyph_vec,
                        });
                    }
                });
                PreparedText {
                    glyph_runs,
                    decorations,
                }
            } else {
                PreparedText {
                    glyph_runs: Vec::new(),
                    decorations: Vec::new(),
                }
            };
            NodeContent::Text(TextDrawPayload {
                node_id: id,
                text: node.text.clone(),
                font_size: node.font_size,
                font_family: node.font_family.clone(),
                font_weight: node.font_weight,
                text_color: node.text_color,
                measured_width: node.last_measured_size.0,
                measured_height: node.last_measured_size.1,
                prepared,
            })
        } else {
            NodeContent::Rect { color: node.color }
        };

        let shadow = if node.shadow_blur > 0.0 {
            Some(ShadowDesc {
                offset_x: node.shadow_offset_x,
                offset_y: node.shadow_offset_y,
                blur: node.shadow_blur,
                color: peniko::Color::from_rgba8(
                    ((node.shadow_color >> 24) & 0xFF) as u8,
                    ((node.shadow_color >> 16) & 0xFF) as u8,
                    ((node.shadow_color >> 8) & 0xFF) as u8,
                    (node.shadow_color & 0xFF) as u8,
                ),
            })
        } else {
            None
        };

        let blur = if node.blur_radius > 0.0 {
            Some(BlurEffect {
                node_id: id,
                local_transform: Transform::IDENTITY,
                width: width as f64,
                height: height as f64,
                blur_radius: node.blur_radius,
                blur_style: node.blur_style,
                opacity: node.opacity,
                overlay_color: node.color,
                border_radius: node.border_radius,
                source_rect: (abs_x, abs_y, width, height),
                deferred_children: node.children.clone(),
            })
        } else {
            None
        };

        out.push(SceneNode {
            id,
            x: x as f64,
            y: y as f64,
            width: width as f64,
            height: height as f64,
            position_x: node.position_x,
            position_y: node.position_y,
            content,
            border_radius: node.border_radius,
            opacity: node.opacity,
            clip_to_bounds: node.clip_to_bounds,
            shadow,
            blur,
            children: node.children.clone(),
        });

        for &child_id in &node.children {
            recurse(state, editors, child_id, (abs_x, abs_y), out);
        }
    }

    let mut nodes = Vec::with_capacity(state.nodes.len());
    if let Some(root) = state.root_id {
        recurse(state, editors, root, (0.0, 0.0), &mut nodes);
    }
    nodes
}

/// CPU-side prepare owned by Runtime: editor lifecycle, text measurement,
/// layout computation. Returns a fully populated RenderPackage.
fn runtime_prepare(
    e: &mut RenderState,
    w: u32,
    h: u32,
) -> RenderPackage {
    if w == 0 || h == 0 {
        let epoch = e.layout_epoch.load(std::sync::atomic::Ordering::Relaxed);
        return RenderPackage {
            viewport: (w, h),
            root_id: None,
            nodes: Vec::new(),
            layout_epoch: epoch,
            did_layout: false,
            dirty_tracker: dyxel_render_api::DirtyTracker::new(),
            bake_plans: Vec::new(),
            recycle_plans: Vec::new(),
        };
    }

    let viewport_changed = {
        let last = *e.last_viewport_size.lock().unwrap();
        last.0 != w || last.1 != h
    };

    let dirty_has_dirty = {
        let g = e.shared_state.lock().unwrap();
        g.dirty_tracker.has_dirty()
    };

    let editor_stale = {
        let editors = e.editors.lock().unwrap();
        let mut last_gens = e.last_editor_generations.lock().unwrap();
        let mut stale = false;
        for (&id, editor) in editors.iter() {
            let current_gen = editor.generation();
            if last_gens.get(&id).map_or(true, |g| *g != current_gen) {
                stale = true;
                last_gens.insert(id, current_gen);
            }
        }
        stale
    };

    let needs_layout = viewport_changed || dirty_has_dirty || editor_stale;

    let (rid, nodes) = {
        let mut g = e.shared_state.lock().unwrap();
        let mut editors = e.editors.lock().unwrap();

        // First pass: create/update editors for text nodes
        for (&id, node) in &g.nodes {
            if node.view_type == ViewType::Text {
                let editor = editors.entry(id).or_insert_with(|| {
                    let mut ed = dyxel_editor::Editor::new(node.font_size);
                    ed.set_text(&node.text);
                    ed.set_text_color(node.text_color);
                    ed
                });

                if editor.text() != node.text {
                    editor.set_text(&node.text);
                }
            }
        }

        // Remove editors for deleted nodes
        let node_ids: HashSet<u32> = g.nodes.keys().copied().collect();
        editors.retain(|id, _| node_ids.contains(id));

        if needs_layout {
            let taffy_to_id: HashMap<taffy::NodeId, u32> = g
                .nodes
                .iter()
                .filter(|(_, n)| n.view_type == ViewType::Text)
                .map(|(id, n)| (n.taffy_node, *id))
                .collect();

            let mut nodes_to_update: Vec<(u32, f32, f32)> = Vec::new();
            for (&id, node) in &g.nodes {
                if node.view_type == ViewType::Text {
                    if let Some(editor) = editors.get_mut(&id) {
                        editor.set_width(None);
                        let (new_width, new_height) = editor.layout_size();
                        let (old_width, old_height) = node.last_measured_size;

                        if (new_width - old_width).abs() > 0.5
                            || (new_height - old_height).abs() > 0.5
                        {
                            nodes_to_update.push((id, new_width, new_height));
                        }
                    }
                }
            }

            for (id, new_width, new_height) in nodes_to_update {
                if let Some(node_mut) = g.nodes.get_mut(&id) {
                    node_mut.last_measured_size = (new_width, new_height);
                }
                g.mark_dirty(id);
            }

            if let Some(rn) = g.root_id.and_then(|id| g.nodes.get(&id).map(|n| n.taffy_node)) {
                let _ = g.taffy.compute_layout_with_measure(
                    rn,
                    taffy::prelude::Size {
                        width: AvailableSpace::Definite(w as f32),
                        height: AvailableSpace::Definite(h as f32),
                    },
                    |_known_dimensions, _available_space, node_id, _node_context, _style| {
                        if let Some(&editor_id) = taffy_to_id.get(&node_id) {
                            if let Some(editor) = editors.get_mut(&editor_id) {
                                editor.set_width(None);
                                let (lw, lh) = editor.layout_size();
                                return taffy::geometry::Size {
                                    width: lw,
                                    height: lh,
                                };
                            }
                        }
                        taffy::geometry::Size {
                            width: _known_dimensions.width.unwrap_or(0.0),
                            height: _known_dimensions.height.unwrap_or(0.0),
                        }
                    },
                );

                let changed_layout_nodes = g.sync_to_shared_buffer();
                if !changed_layout_nodes.is_empty() {
                    dyxel_shared::layout_sync::register_layout_dirty_nodes(
                        &changed_layout_nodes,
                    );
                }

                if g.should_pre_expand() {
                    if g.auto_expand() {
                        log::info!("Auto-expanded node capacity to {}", g.get_capacity());
                    }
                }

                #[cfg(target_os = "android")]
                {
                    static mut FRAME_COUNTER: u32 = 0;
                    unsafe {
                        FRAME_COUNTER += 1;
                        if FRAME_COUNTER % 300 == 0 {
                            let stats = g.get_stats();
                            log::info!(
                                "[NodeStats] capacity={} active={} free={} usage={:.1}%",
                                stats.capacity,
                                stats.active_count,
                                stats.free_count,
                                (stats.active_count as f32 / stats.capacity as f32) * 100.0
                            );
                        }
                    }
                }
            }
        }

        let nodes = build_scene_snapshot(&g, &mut editors);
        (g.root_id, nodes)
    };

    *e.last_viewport_size.lock().unwrap() = (w, h);

    let layout_epoch = if needs_layout {
        e.layout_epoch.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
    } else {
        e.layout_epoch.load(std::sync::atomic::Ordering::Relaxed)
    };

    // Snapshot dirty tracker for backend raster cache stability tracking
    let dirty_tracker = {
        let g = e.shared_state.lock().unwrap();
        g.dirty_tracker.clone()
    };

    // === Raster Cache Policy (Runtime-owned) ===
    // Runtime decides which nodes are stable enough to bake and which textures to recycle.
    // Backend only executes the resulting plans.
    let (bake_plans, recycle_plans) = {
        let mut cache_guard = e.raster_cache.lock().unwrap();
        if let Some(ref mut cache) = *cache_guard {
            cache.next_frame();
            let ready_nodes = cache.update_stability(&dirty_tracker);
            let node_map: std::collections::HashMap<u32, &dyxel_render_api::SceneNode> =
                nodes.iter().map(|n| (n.id, n)).collect();
            let bakes = ready_nodes
                .into_iter()
                .filter_map(|node_id| {
                    node_map.get(&node_id).map(|node| {
                        let tex_w = node.width.ceil() as u32;
                        let tex_h = node.height.ceil() as u32;
                        dyxel_render_api::BakePlan {
                            node_id,
                            width: tex_w,
                            height: tex_h,
                        }
                    })
                })
                .collect();
            let recycles: Vec<dyxel_render_api::RecyclePlan> = cache
                .recycle_unused()
                .into_iter()
                .chain(cache.check_memory_pressure().into_iter())
                .map(|(node_id, texture_id)| dyxel_render_api::RecyclePlan {
                    node_id,
                    texture_id,
                })
                .collect();
            (bakes, recycles)
        } else {
            (Vec::new(), Vec::new())
        }
    };

    RenderPackage {
        viewport: (w, h),
        root_id: rid,
        nodes,
        layout_epoch,
        did_layout: needs_layout,
        dirty_tracker,
        bake_plans,
        recycle_plans,
    }
}

pub fn render_frame(e: &mut RenderState, s: &mut dyn SurfaceState) {
    if let Some(v_ctx) = e.context.downcast_ref::<vello::util::RenderContext>() {
        let device = &v_ctx.devices[0].device;
        let queue = &v_ctx.devices[0].queue;

        let device_handle = DeviceHandle::new(device);
        let queue_handle = QueueHandle::new(queue);

        log::trace!(
            "renderer: Starting frame render, surface size: {}x{}",
            s.width(),
            s.height()
        );

        // === Phase 1: CPU-side prepare (owned by Runtime) ===
        let prepare_start = std::time::Instant::now();
        let package = runtime_prepare(e, s.width(), s.height());
        let prepare_ms = prepare_start.elapsed().as_secs_f64() * 1000.0;
        if prepare_ms > 1.0 {
            log::debug!("[RenderPackage] prepare took {:.2}ms", prepare_ms);
        }

        // === Phase 2: GPU render (owned by Backend) ===
        let render_result = e.backend.render_package(
            device_handle,
            queue_handle,
            s,
            &package,
        );

        if let Err(err) = render_result {
            log::error!("renderer: Render error: {:?}", err);
        }
        // Note: dirty tracker is cleared by Logic Thread (bridge.rs) after sending RequestDraw.
    } else {
        log::error!("renderer: Failed to downcast RenderContext to Vello context");
    }
}
