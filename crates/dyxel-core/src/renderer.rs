// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::engine::{LogicState, RenderState};
use dyxel_render_api::{
    BlurEffect, DirtyField, NodeContent, PreparedText, RenderPackage, RuntimeSurfaceId, SceneNode,
    ShadowDesc, TextDecoration, TextDrawPayload, TextGlyph, TextGlyphRun, Transform,
};
use dyxel_shared::ViewType;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use taffy::style::AvailableSpace;

/// Rebuild a single SceneNode's render-only properties, reusing cached layout values.
/// Called during incremental snapshot updates when layout hasn't changed.
fn rebuild_single_node(
    state: &dyxel_shared::SharedState,
    editors: &mut std::collections::HashMap<u32, dyxel_editor::Editor>,
    node_id: u32,
    cached: &SceneNode,
) -> Option<SceneNode> {
    let node = state.nodes.get(&node_id)?;

    let content = if node.view_type == dyxel_shared::ViewType::Text {
        let prepared = if let Some(editor) = editors.get_mut(&node_id) {
            let mut glyph_runs = Vec::new();
            let mut decorations = Vec::new();
            editor.draw_with_callback(|cmd| match cmd {
                dyxel_editor::DrawCommand::FillRect {
                    x,
                    y,
                    width,
                    height,
                    color,
                } => {
                    decorations.push(TextDecoration {
                        x,
                        y,
                        width,
                        height,
                        color,
                    });
                }
                dyxel_editor::DrawCommand::DrawGlyphs {
                    font_data,
                    font_size,
                    color,
                    glyphs,
                } => {
                    let glyph_vec = glyphs
                        .into_iter()
                        .map(|g| TextGlyph {
                            id: g.id,
                            x: g.x,
                            y: g.y,
                        })
                        .collect();
                    let font_data: std::sync::Arc<dyn std::any::Any + Send + Sync> =
                        std::sync::Arc::new(font_data);
                    glyph_runs.push(TextGlyphRun {
                        font_data,
                        font_size,
                        color,
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
            node_id,
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
            color: [
                ((node.shadow_color >> 16) & 0xFF) as u8,
                ((node.shadow_color >> 8) & 0xFF) as u8,
                (node.shadow_color & 0xFF) as u8,
                ((node.shadow_color >> 24) & 0xFF) as u8,
            ],
        })
    } else {
        None
    };

    let blur = if node.blur_radius > 0.0 {
        Some(BlurEffect {
            node_id,
            local_transform: Transform::IDENTITY,
            width: cached.width,
            height: cached.height,
            blur_radius: node.blur_radius,
            blur_style: node.blur_style,
            opacity: node.opacity,
            overlay_color: node.color,
            border_radius: node.border_radius,
            source_rect: cached.blur.as_ref().map(|b| b.source_rect).unwrap_or((
                cached.x as f32,
                cached.y as f32,
                cached.width as f32,
                cached.height as f32,
            )),
            deferred_children: node.children.clone(),
        })
    } else {
        None
    };

    Some(SceneNode {
        id: node_id,
        x: cached.x,
        y: cached.y,
        width: cached.width,
        height: cached.height,
        position_x: node.position_x,
        position_y: node.position_y,
        content,
        border_radius: node.border_radius,
        opacity: node.opacity,
        clip_to_bounds: node.clip_to_bounds,
        shadow,
        blur,
        children: node.children.clone(),
    })
}

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
        let Some(node) = state.nodes.get(&id) else {
            return;
        };
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
                    dyxel_editor::DrawCommand::FillRect {
                        x,
                        y,
                        width,
                        height,
                        color,
                    } => {
                        decorations.push(TextDecoration {
                            x,
                            y,
                            width,
                            height,
                            color,
                        });
                    }
                    dyxel_editor::DrawCommand::DrawGlyphs {
                        font_data,
                        font_size,
                        color,
                        glyphs,
                    } => {
                        let glyph_vec = glyphs
                            .into_iter()
                            .map(|g| TextGlyph {
                                id: g.id,
                                x: g.x,
                                y: g.y,
                            })
                            .collect();
                        let font_data: std::sync::Arc<dyn std::any::Any + Send + Sync> =
                            std::sync::Arc::new(font_data);
                        glyph_runs.push(TextGlyphRun {
                            font_data,
                            font_size,
                            color,
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
                color: [
                    ((node.shadow_color >> 16) & 0xFF) as u8,
                    ((node.shadow_color >> 8) & 0xFF) as u8,
                    (node.shadow_color & 0xFF) as u8,
                    ((node.shadow_color >> 24) & 0xFF) as u8,
                ],
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

/// CPU-side prepare owned by Logic Worker: editor lifecycle, text measurement,
/// layout computation. Returns a fully populated RenderPackage.
pub fn runtime_prepare(e: &mut LogicState, w: u32, h: u32) -> RenderPackage {
    let runtime_prepare_start = std::time::Instant::now();
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
        let last = *e.last_layout_viewport.lock().unwrap();
        last.0 != w || last.1 != h
    };

    let dirty_has_layout_dirty = {
        let g = e.shared_state.lock().unwrap();
        let layout_bits = DirtyField::Position.bits()
            | DirtyField::Size.bits()
            | DirtyField::Children.bits()
            | DirtyField::Layout.bits()
            | DirtyField::Text.bits();
        g.dirty_tracker
            .node_dirty_fields
            .values()
            .any(|&fields| fields & layout_bits != 0)
    };

    let editor_generation_scan_ms = {
        let editor_gen_scan_start = std::time::Instant::now();
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
        let ms = editor_gen_scan_start.elapsed().as_secs_f64() * 1000.0;
        drop(editors);
        drop(last_gens);
        (stale, ms)
    };

    let needs_layout = viewport_changed || dirty_has_layout_dirty || editor_generation_scan_ms.0;

    let state_build_start = std::time::Instant::now();
    let (rid, nodes) = {
        let mut g = e.shared_state.lock().unwrap();
        let mut editors = e.editors.lock().unwrap();

        // Phase 1: create editors for new text nodes
        let editor_create_start = std::time::Instant::now();
        let shared_font_cx = e.font_context.lock().unwrap().clone();
        for (&id, node) in &g.nodes {
            if node.view_type == ViewType::Text {
                editors.entry(id).or_insert_with(|| {
                    let mut ed = dyxel_editor::Editor::new(node.font_size, shared_font_cx.clone());
                    ed.set_text(&node.text);
                    ed.set_text_color(node.text_color);
                    ed
                });
            }
        }
        let editor_create_update_ms = editor_create_start.elapsed().as_secs_f64() * 1000.0;

        // Phase 2: sync text and style changes into existing editors
        let editor_text_start = std::time::Instant::now();
        for (&id, node) in &g.nodes {
            if node.view_type == ViewType::Text {
                if let Some(editor) = editors.get_mut(&id) {
                    if editor.text() != node.text {
                        editor.set_text(&node.text);
                    }
                    if editor.font_size() != node.font_size {
                        editor.set_font_size(node.font_size);
                    }
                    if editor.text_color() != node.text_color {
                        editor.set_text_color(node.text_color);
                    }
                    if editor.font_weight() != node.font_weight {
                        editor.set_font_weight(node.font_weight);
                    }
                    if editor.font_family() != node.font_family {
                        editor.set_font_family(&node.font_family);
                    }
                }
            }
        }
        let editor_text_compare_ms = editor_text_start.elapsed().as_secs_f64() * 1000.0;

        // Phase 3: remove editors for deleted nodes
        let editor_retain_start = std::time::Instant::now();
        // Avoid temporary HashSet allocation — check directly against g.nodes.
        editors.retain(|id, _| g.nodes.contains_key(id));
        let editor_retain_ms = editor_retain_start.elapsed().as_secs_f64() * 1000.0;

        let mut text_measure_ms = 0.0f64;
        let mut taffy_layout_ms = 0.0f64;
        let mut sync_shared_ms = 0.0f64;
        let mut auto_expand_ms = 0.0f64;

        if needs_layout {
            let taffy_to_id: HashMap<taffy::NodeId, u32> = g
                .nodes
                .iter()
                .filter(|(_, n)| n.view_type == ViewType::Text)
                .map(|(id, n)| (n.taffy_node, *id))
                .collect();

            let editor_layout_size_prepass_start = std::time::Instant::now();
            let mut nodes_to_update = e.nodes_to_update_buffer.lock().unwrap();
            nodes_to_update.clear();
            // Only measure text nodes whose content actually changed (dirty Text).
            // Unchanged text nodes reuse last_measured_size from the previous frame.
            for node_id in g.dirty_tracker.iter_dirty_nodes() {
                if let Some(node) = g.nodes.get(&node_id) {
                    if node.view_type == ViewType::Text {
                        if let Some(editor) = editors.get_mut(&node_id) {
                            editor.set_width(None);
                            let (new_width, new_height) = editor.layout_size();
                            let (old_width, old_height) = node.last_measured_size;

                            if (new_width - old_width).abs() > 0.5
                                || (new_height - old_height).abs() > 0.5
                            {
                                nodes_to_update.push((node_id, new_width, new_height));
                            }
                        }
                    }
                }
            }
            let editor_layout_size_prepass_ms =
                editor_layout_size_prepass_start.elapsed().as_secs_f64() * 1000.0;
            text_measure_ms = editor_layout_size_prepass_ms;

            // Drain the buffer so the lock can be released before mark_dirty loop.
            let updates: Vec<(u32, f32, f32)> = nodes_to_update.drain(..).collect();
            drop(nodes_to_update);

            for (id, new_width, new_height) in updates {
                if let Some(node_mut) = g.nodes.get_mut(&id) {
                    node_mut.last_measured_size = (new_width, new_height);
                }
                g.mark_dirty(id);
            }

            if let Some(rn) = g
                .root_id
                .and_then(|id| g.nodes.get(&id).map(|n| n.taffy_node))
            {
                let taffy_layout_start = std::time::Instant::now();
                // When layout runs, text constraints may have changed even for
                // non-dirty nodes (viewport resize, parent width change, flex
                // redistribution). Measure all text nodes via Editor to avoid
                // stale cached sizes.
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
                taffy_layout_ms = taffy_layout_start.elapsed().as_secs_f64() * 1000.0;

                let sync_shared_start = std::time::Instant::now();
                let changed_layout_nodes = g.sync_to_shared_buffer();
                if !changed_layout_nodes.is_empty() {
                    dyxel_shared::layout_sync::register_layout_dirty_nodes(&changed_layout_nodes);
                }
                sync_shared_ms = sync_shared_start.elapsed().as_secs_f64() * 1000.0;

                let auto_expand_start = std::time::Instant::now();
                if g.should_pre_expand() {
                    if g.auto_expand() {
                        log::info!("Auto-expanded node capacity to {}", g.get_capacity());
                    }
                }
                auto_expand_ms = auto_expand_start.elapsed().as_secs_f64() * 1000.0;

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

        let snapshot_start = std::time::Instant::now();
        let cache_empty = e.scene_nodes_cache.lock().unwrap().is_empty();
        let nodes = if needs_layout || cache_empty {
            let n = build_scene_snapshot(&g, &mut editors);
            // Update cache for incremental updates next frame
            let mut cache = e.scene_nodes_cache.lock().unwrap();
            let mut index = e.scene_node_id_to_index.lock().unwrap();
            *cache = n.clone();
            index.clear();
            for (idx, node) in n.iter().enumerate() {
                index.insert(node.id, idx);
            }
            n
        } else {
            let mut cache = e.scene_nodes_cache.lock().unwrap();
            let index = e.scene_node_id_to_index.lock().unwrap();
            let mut nodes = cache.clone();
            for node_id in g.dirty_tracker.iter_dirty_nodes() {
                if let Some(&idx) = index.get(&node_id) {
                    if let Some(cached) = cache.get(idx) {
                        if let Some(new_node) =
                            rebuild_single_node(&g, &mut editors, node_id, cached)
                        {
                            cache[idx] = new_node.clone();
                            nodes[idx] = new_node;
                        }
                    }
                }
            }
            nodes
        };
        let scene_snapshot_ms = snapshot_start.elapsed().as_secs_f64() * 1000.0;
        let state_build_total_ms = state_build_start.elapsed().as_secs_f64() * 1000.0;
        if state_build_total_ms > 8.0 {
            log::info!(
                "DIAG RuntimePrepareStateBuild={:.2}ms GenScan={:.2}ms EditorCreate={:.2}ms EditorTextCmp={:.2}ms EditorRetain={:.2}ms LayoutSizePrepass={:.2}ms TaffyLayout={:.2}ms SyncShared={:.2}ms AutoExpand={:.2}ms SceneSnapshot={:.2}ms NeedsLayout={}",
                state_build_total_ms,
                editor_generation_scan_ms.1,
                editor_create_update_ms,
                editor_text_compare_ms,
                editor_retain_ms,
                text_measure_ms,
                taffy_layout_ms,
                sync_shared_ms,
                auto_expand_ms,
                scene_snapshot_ms,
                needs_layout
            );
        }
        (g.root_id, nodes)
    };

    *e.last_layout_viewport.lock().unwrap() = (w, h);

    let layout_epoch = if needs_layout {
        e.layout_epoch
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1
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
    let raster_cache_start = std::time::Instant::now();
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
    let raster_cache_ms = raster_cache_start.elapsed().as_secs_f64() * 1000.0;
    let runtime_prepare_total_ms = runtime_prepare_start.elapsed().as_secs_f64() * 1000.0;
    if should_log_runtime_prepare_diag(runtime_prepare_total_ms) {
        log::info!(
            "DIAG RuntimePrepareTotal={:.2}ms RasterCache={:.2}ms BakePlans={} RecyclePlans={} NeedsLayout={} Nodes={}",
            runtime_prepare_total_ms,
            raster_cache_ms,
            bake_plans.len(),
            recycle_plans.len(),
            needs_layout,
            nodes.len()
        );
    }

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

/// Per-frame timing sample for rolling-window diagnostics.
struct RenderFrameSample {
    begin_ms: f64,
    backend_ms: f64,
    end_ms: f64,
}

/// Rolling window of render-frame timing samples.
struct RenderFrameStatsWindow {
    samples: Vec<RenderFrameSample>,
    frame_count: u64,
}

impl RenderFrameStatsWindow {
    const WINDOW_SIZE: usize = 30;

    fn record(&mut self, begin_ms: f64, backend_ms: f64, end_ms: f64) {
        if self.samples.len() >= Self::WINDOW_SIZE {
            self.samples.remove(0);
        }
        self.samples.push(RenderFrameSample {
            begin_ms,
            backend_ms,
            end_ms,
        });
        self.frame_count += 1;
    }

    fn should_report(&self) -> bool {
        self.frame_count % Self::WINDOW_SIZE as u64 == 0 && !self.samples.is_empty()
    }

    fn report(&self) -> Option<String> {
        if self.samples.is_empty() {
            return None;
        }
        let mut begins: Vec<f64> = self.samples.iter().map(|s| s.begin_ms).collect();
        let mut backends: Vec<f64> = self.samples.iter().map(|s| s.backend_ms).collect();
        let mut ends: Vec<f64> = self.samples.iter().map(|s| s.end_ms).collect();
        begins.sort_by(|a, b| a.partial_cmp(b).unwrap());
        backends.sort_by(|a, b| a.partial_cmp(b).unwrap());
        ends.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let p50 = |v: &[f64]| v[v.len() / 2];
        let p95 = |v: &[f64]| {
            let idx = ((v.len() - 1) as f64 * 0.95) as usize;
            v[idx.min(v.len() - 1)]
        };
        let max = |v: &[f64]| v[v.len() - 1];

        Some(format!(
            "[DIAG-RENDERER] window={} begin(p50={:.2} p95={:.2} max={:.2}) backend(p50={:.2} p95={:.2} max={:.2}) end(p50={:.2} p95={:.2} max={:.2})",
            self.samples.len(),
            p50(&begins),
            p95(&begins),
            max(&begins),
            p50(&backends),
            p95(&backends),
            max(&backends),
            p50(&ends),
            p95(&ends),
            max(&ends),
        ))
    }
}

static RENDER_FRAME_STATS: std::sync::Mutex<RenderFrameStatsWindow> =
    std::sync::Mutex::new(RenderFrameStatsWindow {
        samples: Vec::new(),
        frame_count: 0,
    });
static RUNTIME_PREPARE_DIAG_COUNTER: AtomicU64 = AtomicU64::new(0);
const RUNTIME_PREPARE_DIAG_SAMPLE_EVERY_N: u64 = 60;
const RUNTIME_PREPARE_DIAG_SAMPLE_THRESHOLD_MS: f64 = 8.0;
const RUNTIME_PREPARE_DIAG_ALWAYS_THRESHOLD_MS: f64 = 16.0;
static RENDER_SINGLE_FRAME_DIAG_COUNTER: AtomicU64 = AtomicU64::new(0);
const RENDER_SINGLE_FRAME_SAMPLE_EVERY_N: u64 = 60;
const RENDER_BEGIN_SAMPLE_THRESHOLD_MS: f64 = 2.0;
const RENDER_END_SAMPLE_THRESHOLD_MS: f64 = 2.0;
const RENDER_BACKEND_SAMPLE_THRESHOLD_MS: f64 = 15.0;
const RENDER_BEGIN_ALWAYS_THRESHOLD_MS: f64 = 4.0;
const RENDER_END_ALWAYS_THRESHOLD_MS: f64 = 8.0;
const RENDER_BACKEND_ALWAYS_THRESHOLD_MS: f64 = 18.0;

fn should_log_runtime_prepare_diag(ms: f64) -> bool {
    ms >= RUNTIME_PREPARE_DIAG_ALWAYS_THRESHOLD_MS
        || (ms >= RUNTIME_PREPARE_DIAG_SAMPLE_THRESHOLD_MS
            && RUNTIME_PREPARE_DIAG_COUNTER.fetch_add(1, Ordering::Relaxed)
                % RUNTIME_PREPARE_DIAG_SAMPLE_EVERY_N
                == 0)
}

fn should_log_single_render_frame(begin_ms: f64, backend_ms: f64, end_ms: f64) -> bool {
    begin_ms > RENDER_BEGIN_ALWAYS_THRESHOLD_MS
        || backend_ms > RENDER_BACKEND_ALWAYS_THRESHOLD_MS
        || end_ms > RENDER_END_ALWAYS_THRESHOLD_MS
        || ((begin_ms > RENDER_BEGIN_SAMPLE_THRESHOLD_MS
            || backend_ms > RENDER_BACKEND_SAMPLE_THRESHOLD_MS
            || end_ms > RENDER_END_SAMPLE_THRESHOLD_MS)
            && RENDER_SINGLE_FRAME_DIAG_COUNTER.fetch_add(1, Ordering::Relaxed)
                % RENDER_SINGLE_FRAME_SAMPLE_EVERY_N
                == 0)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RenderFrameTimings {
    pub begin_ms: f64,
    pub backend_ms: f64,
    pub end_ms: f64,
}

#[cfg(target_os = "android")]
fn env_flag_enabled(name: &str, default: bool) -> bool {
    std::env::var(name)
        .map(|value| !matches!(value.as_str(), "0" | "false" | "FALSE" | "no" | "NO"))
        .unwrap_or(default)
}

#[cfg(target_os = "android")]
fn env_f64(name: &str, default: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(default)
}

/// Android HWC logs `WaitGpuAcquireFence` when the app presents a surface
/// buffer immediately after queue submission and SurfaceFlinger inherits the
/// still-open GPU fence. Fully waiting for GPU ready eliminates those logs but
/// destroys frame pacing. This experimental path tries a tiny headroom-gated
/// wait, but remains opt-in because real-device validation showed it can move
/// presents closer to SurfaceFlinger composition and increase HWC wait counts.
#[cfg(target_os = "android")]
fn maybe_adaptive_android_present_wait(
    frame: &dyn dyxel_render_api::BackendFrameContext,
    render_elapsed_ms: f64,
    frame_interval_ms: f64,
) -> f64 {
    if !env_flag_enabled("DYXEL_ANDROID_ADAPTIVE_PRESENT_WAIT", false) {
        return 0.0;
    }

    let max_wait_ms = env_f64("DYXEL_ANDROID_ADAPTIVE_PRESENT_WAIT_MAX_MS", 2.0).clamp(0.0, 8.0);
    if max_wait_ms <= 0.0 {
        return 0.0;
    }

    // Keep enough slack for surface_texture.present(), thread wake jitter, and
    // scheduler bookkeeping. This is intentionally conservative: the mitigation
    // must never regress the known-good 60fps default path.
    let headroom_guard_ms =
        env_f64("DYXEL_ANDROID_ADAPTIVE_PRESENT_WAIT_GUARD_MS", 2.5).clamp(0.5, 8.0);
    let interval_budget_ms = if (8.0..=100.0).contains(&frame_interval_ms) {
        frame_interval_ms
    } else {
        16.67
    };
    let available_ms = interval_budget_ms - headroom_guard_ms - render_elapsed_ms;
    let wait_budget_ms = max_wait_ms.min(available_ms.max(0.0));
    if wait_budget_ms < 0.25 {
        return 0.0;
    }

    let wait_t0 = std::time::Instant::now();
    let ready = match frame
        .wait_until_gpu_ready(std::time::Duration::from_secs_f64(wait_budget_ms / 1000.0))
    {
        Ok(ready) => ready,
        Err(err) => {
            log::warn!("[DIAG-RENDERER] adaptive_present_wait error: {:?}", err);
            false
        }
    };
    let wait_ms = wait_t0.elapsed().as_secs_f64() * 1000.0;

    static ADAPTIVE_WAIT_LOG_COUNTER: AtomicU64 = AtomicU64::new(0);
    let sample = ADAPTIVE_WAIT_LOG_COUNTER.fetch_add(1, Ordering::Relaxed) % 60 == 0;
    if sample || ready {
        log::info!(
            "[DIAG-RENDERER] adaptive_present_wait wait={:.2}ms budget={:.2}ms ready={} render_elapsed={:.2}ms interval={:.2}ms",
            wait_ms,
            wait_budget_ms,
            ready,
            render_elapsed_ms,
            interval_budget_ms
        );
    }

    wait_ms
}

pub struct DeferredRenderFrame {
    runtime: Arc<StdMutex<Box<dyn dyxel_render_api::GraphicsRuntime>>>,
    frame: Box<dyn dyxel_render_api::BackendFrameContext>,
    begin_ms: f64,
    backend_ms: f64,
}

impl DeferredRenderFrame {
    pub fn submitted_timings(&self) -> RenderFrameTimings {
        RenderFrameTimings {
            begin_ms: self.begin_ms,
            backend_ms: self.backend_ms,
            end_ms: 0.0,
        }
    }

    pub fn wait_until_gpu_ready(&self, timeout: std::time::Duration) -> (bool, f64) {
        let t0 = std::time::Instant::now();
        let ready = match self.frame.wait_until_gpu_ready(timeout) {
            Ok(ready) => ready,
            Err(err) => {
                log::warn!("renderer: GPU ready wait failed: {:?}", err);
                false
            }
        };
        (ready, t0.elapsed().as_secs_f64() * 1000.0)
    }

    pub fn present(self) -> RenderFrameTimings {
        let DeferredRenderFrame {
            runtime,
            frame,
            begin_ms,
            backend_ms,
        } = self;

        let t2 = std::time::Instant::now();
        let end_result = if frame.supports_detached_present() {
            frame.present_detached().map(|_| ())
        } else {
            runtime.lock().unwrap().end_frame(frame)
        };
        if let Err(err) = end_result {
            log::error!("renderer: deferred present failed: {:?}", err);
        }
        let end_ms = t2.elapsed().as_secs_f64() * 1000.0;
        record_render_frame_timing(begin_ms, backend_ms, end_ms)
    }
}

/// Render into an offscreen frame context and return it for a bounded presenter.
pub fn render_frame_with_package_deferred_present(
    e: &mut RenderState,
    surface_id: RuntimeSurfaceId,
    package: &RenderPackage,
    frame_timing: Option<(f64, f64)>,
    perf_stats: Option<dyxel_perf::FramePerformanceStats>,
) -> Option<DeferredRenderFrame> {
    let Some(runtime_arc) = e.runtime.as_ref() else {
        log::error!("renderer: GraphicsRuntime not available");
        return None;
    };
    let Some(backend) = e.backend_v2.as_mut() else {
        log::error!("renderer: RenderBackendV2 not available");
        return None;
    };

    if let Some((pacer_wait_ms, frame_interval_ms)) = frame_timing {
        backend.set_frame_timing(pacer_wait_ms, frame_interval_ms);
    }

    if let Some(stats) = perf_stats {
        backend.set_frame_performance_stats(stats);
    }

    let t0 = std::time::Instant::now();
    let mut frame = match runtime_arc.lock().unwrap().begin_frame(surface_id) {
        Ok(f) => f,
        Err(err) => {
            log::error!("renderer: begin_frame failed: {:?}", err);
            return None;
        }
    };
    let begin_ms = t0.elapsed().as_secs_f64() * 1000.0;

    let t1 = std::time::Instant::now();
    let render_result = backend.render(frame.as_mut(), package);
    let backend_ms = t1.elapsed().as_secs_f64() * 1000.0;

    if let Err(err) = render_result {
        log::error!("renderer: Render error: {:?}", err);
    }

    Some(DeferredRenderFrame {
        runtime: runtime_arc.clone(),
        frame,
        begin_ms,
        backend_ms,
    })
}

/// GPU-side render owned by Render Worker.
/// Consumes a pre-prepared RenderPackage snapshot from the mailbox.
pub fn render_frame_with_package(
    e: &mut RenderState,
    surface_id: RuntimeSurfaceId,
    package: &RenderPackage,
    frame_timing: Option<(f64, f64)>,
    perf_stats: Option<dyxel_perf::FramePerformanceStats>,
) -> Option<RenderFrameTimings> {
    let Some(runtime_arc) = e.runtime.as_ref() else {
        log::error!("renderer: GraphicsRuntime not available");
        return None;
    };
    let Some(backend) = e.backend_v2.as_mut() else {
        log::error!("renderer: RenderBackendV2 not available");
        return None;
    };

    if let Some((pacer_wait_ms, frame_interval_ms)) = frame_timing {
        backend.set_frame_timing(pacer_wait_ms, frame_interval_ms);
    }

    if let Some(stats) = perf_stats {
        backend.set_frame_performance_stats(stats);
    }

    let t0 = std::time::Instant::now();
    let mut frame = match runtime_arc.lock().unwrap().begin_frame(surface_id) {
        Ok(f) => f,
        Err(err) => {
            log::error!("renderer: begin_frame failed: {:?}", err);
            return None;
        }
    };
    let begin_ms = t0.elapsed().as_secs_f64() * 1000.0;

    let t1 = std::time::Instant::now();
    let render_result = backend.render(frame.as_mut(), package);
    let backend_ms = t1.elapsed().as_secs_f64() * 1000.0;

    if let Err(err) = render_result {
        log::error!("renderer: Render error: {:?}", err);
    }

    let t2 = std::time::Instant::now();
    #[cfg(target_os = "android")]
    if let Some((_, frame_interval_ms)) = frame_timing {
        let render_elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let _ = maybe_adaptive_android_present_wait(
            frame.as_ref(),
            render_elapsed_ms,
            frame_interval_ms,
        );
    }
    if let Err(err) = runtime_arc.lock().unwrap().end_frame(frame) {
        log::error!("renderer: end_frame failed: {:?}", err);
    }
    let end_ms = t2.elapsed().as_secs_f64() * 1000.0;

    Some(record_render_frame_timing(begin_ms, backend_ms, end_ms))
}

fn record_render_frame_timing(begin_ms: f64, backend_ms: f64, end_ms: f64) -> RenderFrameTimings {
    // Per-frame threshold trigger. Keep this sampled: present/end_frame often
    // sits around 1–3ms on Android, and logging every such frame adds enough
    // logcat pressure to perturb the cadence we are measuring.
    if should_log_single_render_frame(begin_ms, backend_ms, end_ms) {
        log::info!(
            "[DIAG-RENDERER] single_frame begin={:.2}ms backend={:.2}ms end={:.2}ms",
            begin_ms,
            backend_ms,
            end_ms,
        );
    }

    // Rolling-window p50/p95/max report
    {
        let mut stats = RENDER_FRAME_STATS.lock().unwrap();
        stats.record(begin_ms, backend_ms, end_ms);
        if stats.should_report() {
            if let Some(report) = stats.report() {
                log::info!("{}", report);
            }
        }
    }

    RenderFrameTimings {
        begin_ms,
        backend_ms,
        end_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::SharedState;
    use dyxel_perf::FramePerformanceStats;
    use dyxel_render_api::{
        BackendFrameContext, GraphicsRuntime, LifecycleEvent, NativeSurfaceHandle, RenderBackendV2,
        RenderFrameStats, RuntimeKind, SharedMutex, SharedPtr,
    };
    use std::sync::{Arc, Mutex as StdMutex};

    struct FakeFrameContext;

    impl BackendFrameContext for FakeFrameContext {
        fn as_any(&mut self) -> &mut dyn std::any::Any {
            self
        }

        fn runtime_kind(&self) -> RuntimeKind {
            RuntimeKind::Wgpu
        }
    }

    #[derive(Default)]
    struct BackendObservations {
        timing: Option<(f64, f64)>,
        perf: Option<FramePerformanceStats>,
        render_called: bool,
        end_called: bool,
    }

    struct FakeRuntime {
        observations: Arc<StdMutex<BackendObservations>>,
    }

    impl GraphicsRuntime for FakeRuntime {
        fn initialize(&mut self) -> anyhow::Result<()> {
            Ok(())
        }

        fn create_surface(
            &mut self,
            _handle: NativeSurfaceHandle,
            _width: u32,
            _height: u32,
        ) -> anyhow::Result<RuntimeSurfaceId> {
            Ok(RuntimeSurfaceId(1))
        }

        fn resize_surface(
            &mut self,
            _surface: RuntimeSurfaceId,
            _width: u32,
            _height: u32,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        fn suspend(&mut self) -> anyhow::Result<()> {
            Ok(())
        }

        fn resume(&mut self) -> anyhow::Result<()> {
            Ok(())
        }

        fn sync_gpu(&mut self) -> anyhow::Result<()> {
            Ok(())
        }

        fn begin_frame(
            &mut self,
            _surface: RuntimeSurfaceId,
        ) -> anyhow::Result<Box<dyn BackendFrameContext>> {
            Ok(Box::new(FakeFrameContext))
        }

        fn end_frame(&mut self, _frame: Box<dyn BackendFrameContext>) -> anyhow::Result<()> {
            self.observations.lock().unwrap().end_called = true;
            Ok(())
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    struct FakeBackend {
        observations: Arc<StdMutex<BackendObservations>>,
    }

    impl RenderBackendV2 for FakeBackend {
        fn initialize(&mut self, _runtime: &mut dyn GraphicsRuntime) -> anyhow::Result<()> {
            Ok(())
        }

        fn render(
            &mut self,
            _frame: &mut dyn BackendFrameContext,
            _package: &RenderPackage,
        ) -> anyhow::Result<RenderFrameStats> {
            self.observations.lock().unwrap().render_called = true;
            Ok(RenderFrameStats {
                cpu_time_ms: None,
                gpu_time_ms: None,
                backend_internal_stats: None,
            })
        }

        fn set_frame_timing(&self, pacer_wait_ms: f64, frame_interval_ms: f64) {
            self.observations.lock().unwrap().timing = Some((pacer_wait_ms, frame_interval_ms));
        }

        fn set_frame_performance_stats(&self, stats: FramePerformanceStats) {
            self.observations.lock().unwrap().perf = Some(stats);
        }

        fn on_lifecycle_event(&self, _event: LifecycleEvent) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn render_frame_with_package_propagates_frame_stats_to_backend() {
        let observations = Arc::new(StdMutex::new(BackendObservations::default()));
        let runtime: Box<dyn GraphicsRuntime> = Box::new(FakeRuntime {
            observations: observations.clone(),
        });
        let backend: Box<dyn RenderBackendV2> = Box::new(FakeBackend {
            observations: observations.clone(),
        });

        let mut render_state = RenderState {
            runtime: Some(Arc::new(StdMutex::new(runtime))),
            backend_v2: Some(backend),
            shared_state: SharedPtr::new(SharedMutex::new(SharedState::new())),
        };
        let package = RenderPackage::new((100, 100), None, Vec::new());

        let perf = FramePerformanceStats {
            ui_fps: 23.0,
            raster_fps: 17.0,
            target_fps: 60.0,
            jank_count: 2,
            dropped_count: 1,
            jank_rate: 0.1,
            drop_rate: 0.05,
        };

        render_frame_with_package(
            &mut render_state,
            RuntimeSurfaceId(1),
            &package,
            Some((0.0, 16.67)),
            Some(perf),
        );

        let observations = observations.lock().unwrap();
        assert!(
            observations.render_called,
            "backend render should be called"
        );
        assert!(
            observations.end_called,
            "runtime end_frame should be called"
        );
        assert_eq!(
            observations.timing,
            Some((0.0, 16.67)),
            "backend should receive frame timing before render"
        );
        let observed_perf = observations
            .perf
            .expect("backend should receive scheduler perf stats before render");
        assert_eq!(observed_perf.ui_fps, perf.ui_fps);
        assert_eq!(observed_perf.raster_fps, perf.raster_fps);
        assert_eq!(observed_perf.target_fps, perf.target_fps);
        assert_eq!(observed_perf.jank_count, perf.jank_count);
        assert_eq!(observed_perf.dropped_count, perf.dropped_count);
    }
}
