// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use dyxel_render_api::{
    BackendFrameContext, GraphicsRuntime, NodeContent, RenderBackendV2, RenderFrameStats,
    RenderPackage, RuntimeKind, SceneNode, TextDrawPayload,
};
use impellers::{
    ClipOperation, Color, DisplayList, DisplayListBuilder, FillType, FontWeight, Paint, Paragraph,
    ParagraphBuilder, ParagraphStyle, PathBuilder, Point, Rect, RoundingRadii, Size, TextAlignment,
    TextDirection, TypographyContext,
};
use kurbo::Vec2;
use std::borrow::Cow;
use std::collections::{hash_map::DefaultHasher, HashMap, HashSet};
use std::hash::{Hash, Hasher};

const PARAGRAPH_CACHE_LIMIT: usize = 2048;
const PARAGRAPH_REBUILD_BUDGET_PER_FRAME: u32 = 4;
const PREPARED_FONT_ALIAS: &str = "dyxel-prepared";

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ParagraphCacheKey {
    text: String,
    family: String,
    font_size_bits: u32,
    font_weight: u16,
    color: [u8; 4],
    width_bits: u32,
    font_signature: u64,
}

struct ParagraphCacheEntry {
    key: ParagraphCacheKey,
    paragraph: Paragraph,
}

/// Drawing backend for the experimental Impeller runtime.
pub struct ImpellerDrawingBackend {
    frame_count: u64,
    paragraph_cache: HashMap<u32, ParagraphCacheEntry>,
    text_ready_logged: bool,
    text_failure_logged: bool,
    text_cache_hits: u64,
    text_cache_misses: u64,
    text_cache_deferred: u64,
    text_rebuilds_this_frame: u32,
    scene_probe_logged: bool,
    last_display_list_probe_mode: Option<String>,
}

impl ImpellerDrawingBackend {
    pub fn new() -> Self {
        Self {
            frame_count: 0,
            paragraph_cache: HashMap::new(),
            text_ready_logged: false,
            text_failure_logged: false,
            text_cache_hits: 0,
            text_cache_misses: 0,
            text_cache_deferred: 0,
            text_rebuilds_this_frame: 0,
            scene_probe_logged: false,
            last_display_list_probe_mode: None,
        }
    }
}

impl Default for ImpellerDrawingBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderBackendV2 for ImpellerDrawingBackend {
    fn initialize(&mut self, runtime: &mut dyn GraphicsRuntime) -> anyhow::Result<()> {
        if runtime
            .as_any()
            .downcast_ref::<super::runtime::ImpellerRuntime>()
            .is_none()
        {
            return Err(anyhow::anyhow!(
                "ImpellerDrawingBackend needs ImpellerRuntime"
            ));
        }
        log::info!("[DIAG-IMPELLER] drawing backend initialized");
        Ok(())
    }

    fn render(
        &mut self,
        frame: &mut dyn BackendFrameContext,
        package: &RenderPackage,
    ) -> anyhow::Result<RenderFrameStats> {
        if frame.runtime_kind() != RuntimeKind::Impeller {
            return Err(anyhow::anyhow!(
                "ImpellerDrawingBackend expected Impeller runtime, got {:?}",
                frame.runtime_kind()
            ));
        }
        let frame = frame
            .as_any()
            .downcast_mut::<super::runtime::ImpellerFrameContext>()
            .ok_or_else(|| anyhow::anyhow!("Invalid Impeller frame context"))?;
        let surface = frame
            .surface
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Impeller frame surface missing"))?;

        let build_t0 = std::time::Instant::now();
        let text_hits_before = self.text_cache_hits;
        let text_misses_before = self.text_cache_misses;
        let text_deferred_before = self.text_cache_deferred;
        self.text_rebuilds_this_frame = 0;
        let display_list = self.build_display_list(package)?;
        let build_ms = build_t0.elapsed().as_secs_f64() * 1000.0;
        let text_hits = self.text_cache_hits.saturating_sub(text_hits_before);
        let text_misses = self.text_cache_misses.saturating_sub(text_misses_before);
        let text_deferred = self
            .text_cache_deferred
            .saturating_sub(text_deferred_before);

        let draw_t0 = std::time::Instant::now();
        surface
            .draw_display_list(&display_list)
            .map_err(|err| anyhow::anyhow!("Impeller draw_display_list failed: {}", err))?;
        let draw_ms = draw_t0.elapsed().as_secs_f64() * 1000.0;

        self.frame_count += 1;
        if self.frame_count <= 5 || self.frame_count % 60 == 0 || build_ms + draw_ms >= 20.0 {
            log::info!(
                "[DIAG-IMPELLER] drew frame count={} nodes={} viewport={}x{} build_ms={:.2} draw_ms={:.2} text_hit={} text_miss={} text_defer={} text_cache={}",
                self.frame_count,
                package.nodes.len(),
                package.viewport.0,
                package.viewport.1,
                build_ms,
                draw_ms,
                text_hits,
                text_misses,
                text_deferred,
                self.paragraph_cache.len()
            );
        }

        Ok(RenderFrameStats {
            cpu_time_ms: Some(build_ms + draw_ms),
            gpu_time_ms: None,
            backend_internal_stats: None,
        })
    }
}

impl ImpellerDrawingBackend {
    fn build_display_list(&mut self, package: &RenderPackage) -> anyhow::Result<DisplayList> {
        let probe_mode = display_list_probe_mode();
        if let Some(mode) = probe_mode.as_deref() {
            if matches!(
                mode,
                "mini-none" | "mini-cull" | "mini-normalized" | "mini-unit" | "mini"
            ) {
                self.log_display_list_probe_mode(mode, package);
                return self.build_minimal_display_list_probe(package, mode);
            }
        }

        let cull = Rect::from_size(Size::new(
            package.viewport.0 as f32,
            package.viewport.1 as f32,
        ));
        let use_cull_rect = !matches!(
            probe_mode.as_deref(),
            Some("scene-none" | "scene-no-cull" | "no-cull")
        );
        if let Some(mode) = probe_mode.as_deref() {
            self.log_display_list_probe_mode(mode, package);
        }
        let mut builder = if use_cull_rect {
            DisplayListBuilder::new(Some(&cull))
        } else {
            DisplayListBuilder::new(None)
        };
        let mut clear = Paint::default();
        clear.set_color(Color::new_srgba(0.02, 0.02, 0.025, 1.0));
        builder.draw_paint(&clear);
        if matches!(probe_mode.as_deref(), Some("scene-normalized")) {
            let sx = 1.0 / (package.viewport.0.max(1) as f32);
            let sy = 1.0 / (package.viewport.1.max(1) as f32);
            if self.frame_count == 0 {
                log::info!(
                    "[DIAG-IMPELLER] applying android normalized-root scale sx={:.8} sy={:.8}",
                    sx,
                    sy
                );
            }
            builder.scale(sx, sy);
        }

        let nodes: HashMap<u32, &SceneNode> = package.nodes.iter().map(|n| (n.id, n)).collect();
        if !self.scene_probe_logged {
            self.log_scene_probe(package, &nodes);
            self.scene_probe_logged = true;
        }
        let mut visited = HashSet::new();
        if let Some(root_id) = package.root_id {
            self.draw_node_recursive(root_id, &nodes, &mut builder, Vec2::ZERO, &mut visited);
        } else {
            for node in &package.nodes {
                self.draw_node_recursive(node.id, &nodes, &mut builder, Vec2::ZERO, &mut visited);
            }
        }

        builder
            .build()
            .ok_or_else(|| anyhow::anyhow!("Impeller failed to build display list"))
    }

    fn log_display_list_probe_mode(&mut self, mode: &str, package: &RenderPackage) {
        if self
            .last_display_list_probe_mode
            .as_deref()
            .is_some_and(|last| last == mode)
        {
            return;
        }
        self.last_display_list_probe_mode = Some(mode.to_string());
        log::info!(
            "[DIAG-IMPELLER] display-list probe mode={} viewport={}x{} nodes={}",
            mode,
            package.viewport.0,
            package.viewport.1,
            package.nodes.len()
        );
        log_rect_layout_probe();
    }

    fn build_minimal_display_list_probe(
        &mut self,
        package: &RenderPackage,
        mode: &str,
    ) -> anyhow::Result<DisplayList> {
        let cull = Rect::from_size(Size::new(
            package.viewport.0 as f32,
            package.viewport.1 as f32,
        ));
        let use_cull_rect = mode == "mini-cull";
        let mut builder = if use_cull_rect {
            DisplayListBuilder::new(Some(&cull))
        } else {
            DisplayListBuilder::new(None)
        };

        let mut background = Paint::default();
        background.set_color(Color::new_srgba(0.015, 0.015, 0.02, 1.0));
        builder.draw_paint(&background);
        if mode == "mini-unit" {
            return self.finish_unit_coordinate_probe(builder);
        }
        if mode == "mini-normalized" {
            let sx = 1.0 / (package.viewport.0.max(1) as f32);
            let sy = 1.0 / (package.viewport.1.max(1) as f32);
            builder.scale(sx, sy);
        }

        let mut red = Paint::default();
        red.set_color(Color::new_srgba(1.0, 0.0, 0.0, 1.0));
        let origin_rect = Rect::from_size(Size::new(160.0, 160.0));
        builder.draw_rect(&origin_rect, &red);

        let mut blue = Paint::default();
        blue.set_color(Color::new_srgba(0.0, 0.2, 1.0, 1.0));
        let offset_rect = Rect::new(Point::new(240.0, 40.0), Size::new(160.0, 160.0));
        builder.draw_rect(&offset_rect, &blue);

        let mut green = Paint::default();
        green.set_color(Color::new_srgba(0.0, 1.0, 0.0, 1.0));
        let manual_path = rect_path(40.0, 260.0, 160.0, 160.0);
        builder.draw_path(&manual_path, &green);

        let mut yellow = Paint::default();
        yellow.set_color(Color::new_srgba(1.0, 1.0, 0.0, 1.0));
        builder.save();
        builder.translate(280.0, 260.0);
        let translated_rect = Rect::from_size(Size::new(140.0, 140.0));
        builder.draw_rect(&translated_rect, &yellow);
        builder.restore();

        let mut magenta = Paint::default();
        magenta.set_color(Color::new_srgba(1.0, 0.0, 1.0, 1.0));
        builder.save();
        builder.translate(480.0, 260.0);
        let translated_path = rect_path(0.0, 0.0, 140.0, 140.0);
        builder.draw_path(&translated_path, &magenta);
        builder.restore();

        builder
            .build()
            .ok_or_else(|| anyhow::anyhow!("Impeller failed to build minimal probe display list"))
    }

    fn finish_unit_coordinate_probe(
        &mut self,
        mut builder: DisplayListBuilder,
    ) -> anyhow::Result<DisplayList> {
        let mut red = Paint::default();
        red.set_color(Color::new_srgba(1.0, 0.0, 0.0, 1.0));
        let origin_rect = Rect::from_size(Size::new(0.25, 0.25));
        builder.draw_rect(&origin_rect, &red);

        let mut blue = Paint::default();
        blue.set_color(Color::new_srgba(0.0, 0.2, 1.0, 1.0));
        let offset_rect = Rect::new(Point::new(0.32, 0.05), Size::new(0.25, 0.25));
        builder.draw_rect(&offset_rect, &blue);

        let mut green = Paint::default();
        green.set_color(Color::new_srgba(0.0, 1.0, 0.0, 1.0));
        let manual_path = rect_path(0.05, 0.35, 0.25, 0.25);
        builder.draw_path(&manual_path, &green);

        let mut yellow = Paint::default();
        yellow.set_color(Color::new_srgba(1.0, 1.0, 0.0, 1.0));
        builder.save();
        builder.translate(0.40, 0.35);
        let translated_rect = Rect::from_size(Size::new(0.22, 0.22));
        builder.draw_rect(&translated_rect, &yellow);
        builder.restore();

        let mut magenta = Paint::default();
        magenta.set_color(Color::new_srgba(1.0, 0.0, 1.0, 1.0));
        builder.save();
        builder.translate(0.67, 0.35);
        let translated_path = rect_path(0.0, 0.0, 0.22, 0.22);
        builder.draw_path(&translated_path, &magenta);
        builder.restore();

        builder
            .build()
            .ok_or_else(|| anyhow::anyhow!("Impeller failed to build unit probe display list"))
    }

    fn draw_node_recursive(
        &mut self,
        id: u32,
        nodes: &HashMap<u32, &SceneNode>,
        builder: &mut DisplayListBuilder,
        parent_pos: Vec2,
        visited: &mut HashSet<u32>,
    ) {
        if !visited.insert(id) {
            return;
        }
        let Some(node) = nodes.get(&id).copied() else {
            return;
        };

        let pos_offset = Vec2::new(node.position_x as f64, node.position_y as f64);
        let is_absolute = node.position_x != 0.0 || node.position_y != 0.0;
        let local_pos = if is_absolute {
            pos_offset
        } else {
            Vec2::new(node.x, node.y)
        };
        let global_pos = parent_pos + local_pos;

        builder.save();
        // DisplayListBuilder transforms are stack-relative. The render package
        // stores node positions relative to their parent, while `global_pos` is
        // only used for descendant bookkeeping. Translating by `global_pos`
        // here would double-apply every ancestor and push most nested content
        // off-screen, leaving only the dark root clear/background visible.
        builder.translate(local_pos.x as f32, local_pos.y as f32);
        let local_rect = Rect::from_size(Size::new(node.width as f32, node.height as f32));
        if node.clip_to_bounds {
            builder.clip_rect(&local_rect, ClipOperation::Intersect);
        }

        self.draw_shadow(node, builder);
        self.draw_content(node, builder);

        let child_parent = global_pos;
        for &child in &node.children {
            self.draw_node_recursive(child, nodes, builder, child_parent, visited);
        }
        builder.restore();
    }

    fn draw_shadow(&self, node: &SceneNode, builder: &mut DisplayListBuilder) {
        let Some(shadow) = &node.shadow else {
            return;
        };
        let mut paint = Paint::default();
        paint.set_color(color_with_opacity(shadow.color, node.opacity));
        let rect = Rect::new(
            Point::new(shadow.offset_x, shadow.offset_y),
            Size::new(node.width as f32, node.height as f32),
        );
        draw_rect_or_round(builder, &rect, node.border_radius, &paint);
    }

    fn draw_content(&mut self, node: &SceneNode, builder: &mut DisplayListBuilder) {
        match &node.content {
            NodeContent::Rect { color } => {
                let mut paint = Paint::default();
                paint.set_color(color_with_opacity(*color, node.opacity));
                let rect = Rect::from_size(Size::new(node.width as f32, node.height as f32));
                draw_rect_or_round(builder, &rect, node.border_radius, &paint);
            }
            NodeContent::Text(payload) => {
                self.draw_text(payload, node, builder);
            }
        }

        if let Some(blur) = &node.blur {
            let mut paint = Paint::default();
            paint.set_color(color_with_opacity(
                blur.overlay_color,
                blur.opacity * node.opacity,
            ));
            let rect = Rect::from_size(Size::new(node.width as f32, node.height as f32));
            draw_rect_or_round(
                builder,
                &rect,
                blur.border_radius.max(node.border_radius),
                &paint,
            );
        }
    }

    fn draw_text(
        &mut self,
        payload: &TextDrawPayload,
        node: &SceneNode,
        builder: &mut DisplayListBuilder,
    ) {
        for deco in &payload.prepared.decorations {
            let mut paint = Paint::default();
            paint.set_color(color_with_opacity(deco.color, node.opacity));
            let rect = Rect::new(
                Point::new(deco.x, deco.y),
                Size::new(deco.width, deco.height),
            );
            builder.draw_rect(&rect, &paint);
        }

        if payload.text.is_empty() {
            return;
        }

        let paragraph_width = text_layout_width(payload, node);
        let mut drew_text = false;
        if let Some(paragraph) = self.paragraph_for_payload(payload, paragraph_width, node.opacity)
        {
            builder.draw_paragraph(paragraph, Point::new(0.0, 0.0));
            drew_text = true;
        }

        if drew_text {
            if !self.text_ready_logged {
                log::info!(
                    "[DIAG-IMPELLER] text paragraph drawing enabled cache_limit={} rebuild_budget={}",
                    PARAGRAPH_CACHE_LIMIT,
                    PARAGRAPH_REBUILD_BUDGET_PER_FRAME
                );
                self.text_ready_logged = true;
            }
        } else if !self.text_failure_logged {
            log::warn!("[DIAG-IMPELLER] text paragraph build failed; rendering decorations only");
            self.text_failure_logged = true;
        }
    }

    fn paragraph_for_payload(
        &mut self,
        payload: &TextDrawPayload,
        width: f32,
        opacity: f32,
    ) -> Option<&Paragraph> {
        let effective_color = apply_opacity_to_color(payload.text_color, opacity);
        let font_signature = prepared_font_signature(payload);
        let family = if font_signature != 0 {
            PREPARED_FONT_ALIAS.to_string()
        } else {
            fallback_font_family(&payload.font_family).to_string()
        };
        let key = ParagraphCacheKey {
            text: payload.text.clone(),
            family,
            font_size_bits: payload.font_size.to_bits(),
            font_weight: payload.font_weight,
            color: effective_color,
            width_bits: width.to_bits(),
            font_signature,
        };

        let needs_rebuild = self
            .paragraph_cache
            .get(&payload.node_id)
            .map(|entry| entry.key != key)
            .unwrap_or(true);

        if needs_rebuild {
            if self.paragraph_cache.contains_key(&payload.node_id)
                && self.text_rebuilds_this_frame >= PARAGRAPH_REBUILD_BUDGET_PER_FRAME
            {
                self.text_cache_hits += 1;
                self.text_cache_deferred += 1;
                return self
                    .paragraph_cache
                    .get(&payload.node_id)
                    .map(|entry| &entry.paragraph);
            }
            self.text_cache_misses += 1;
            self.text_rebuilds_this_frame += 1;
            if self.paragraph_cache.len() > PARAGRAPH_CACHE_LIMIT {
                log::info!(
                    "[DIAG-IMPELLER] clearing stale text paragraph cache entries={}",
                    self.paragraph_cache.len()
                );
                self.paragraph_cache.clear();
            }
            let paragraph = build_paragraph(payload, width, effective_color, font_signature)?;
            self.paragraph_cache.insert(
                payload.node_id,
                ParagraphCacheEntry {
                    key: key.clone(),
                    paragraph,
                },
            );
        } else {
            self.text_cache_hits += 1;
        }

        self.paragraph_cache
            .get(&payload.node_id)
            .map(|entry| &entry.paragraph)
    }

    fn log_scene_probe(&self, package: &RenderPackage, nodes: &HashMap<u32, &SceneNode>) {
        log::info!(
            "[DIAG-IMPELLER] scene probe root={:?} nodes={} viewport={}x{}",
            package.root_id,
            package.nodes.len(),
            package.viewport.0,
            package.viewport.1
        );
        if let Some(root_id) = package.root_id {
            if let Some(root) = nodes.get(&root_id).copied() {
                log_node_probe("root", root);
                for (idx, child_id) in root.children.iter().take(8).enumerate() {
                    if let Some(child) = nodes.get(child_id).copied() {
                        log_node_probe(&format!("root_child_{idx}"), child);
                    }
                }
            }
        }
    }
}

fn rect_path(x: f32, y: f32, width: f32, height: f32) -> impellers::Path {
    let mut builder = PathBuilder::default();
    builder
        .move_to(Point::new(x, y))
        .line_to(Point::new(x + width, y))
        .line_to(Point::new(x + width, y + height))
        .line_to(Point::new(x, y + height))
        .close();
    builder.take_path_new(FillType::NonZero)
}

fn log_rect_layout_probe() {
    let rect = Rect::new(Point::new(1.0, 2.0), Size::new(3.0, 4.0));
    let point = Point::new(5.0, 6.0);
    let size = Size::new(7.0, 8.0);
    let rect_words: [u32; 4] = unsafe { std::mem::transmute(rect) };
    let point_words: [u32; 2] = unsafe { std::mem::transmute(point) };
    let size_words: [u32; 2] = unsafe { std::mem::transmute(size) };
    log::info!(
        "[DIAG-IMPELLER] rust layout Rect size={} align={} bits={:08x},{:08x},{:08x},{:08x} Point size={} bits={:08x},{:08x} Size size={} bits={:08x},{:08x}",
        std::mem::size_of::<Rect>(),
        std::mem::align_of::<Rect>(),
        rect_words[0],
        rect_words[1],
        rect_words[2],
        rect_words[3],
        std::mem::size_of::<Point>(),
        point_words[0],
        point_words[1],
        std::mem::size_of::<Size>(),
        size_words[0],
        size_words[1]
    );
}

#[cfg(target_os = "android")]
fn display_list_probe_mode() -> Option<String> {
    use std::ffi::{CStr, CString};

    let key = CString::new("debug.dyxel.impeller_probe").ok()?;
    let mut value = [0 as libc::c_char; libc::PROP_VALUE_MAX as usize];
    let len = unsafe { libc::__system_property_get(key.as_ptr(), value.as_mut_ptr()) };
    if len <= 0 {
        return std::env::var("DYXEL_IMPELLER_DISPLAY_LIST_PROBE")
            .ok()
            .and_then(normalize_probe_mode);
    }
    let value = unsafe { CStr::from_ptr(value.as_ptr()) }
        .to_string_lossy()
        .to_string();
    normalize_probe_mode(value)
}

#[cfg(not(target_os = "android"))]
fn display_list_probe_mode() -> Option<String> {
    std::env::var("DYXEL_IMPELLER_DISPLAY_LIST_PROBE")
        .ok()
        .and_then(normalize_probe_mode)
}

fn normalize_probe_mode(value: impl AsRef<str>) -> Option<String> {
    let mode = value.as_ref().trim().to_ascii_lowercase();
    if mode.is_empty() || matches!(mode.as_str(), "0" | "false" | "off" | "none") {
        None
    } else {
        Some(mode)
    }
}

fn log_node_probe(label: &str, node: &SceneNode) {
    let content = match &node.content {
        NodeContent::Rect { color } => format!("rect rgba={color:?}"),
        NodeContent::Text(payload) => format!(
            "text len={} color={:?} measured={:.1}x{:.1}",
            payload.text.len(),
            payload.text_color,
            payload.measured_width,
            payload.measured_height
        ),
    };
    log::info!(
        "[DIAG-IMPELLER] node {label} id={} xy=({:.1},{:.1}) pos=({:.1},{:.1}) wh=({:.1},{:.1}) opacity={:.2} clip={} children={} {}",
        node.id,
        node.x,
        node.y,
        node.position_x,
        node.position_y,
        node.width,
        node.height,
        node.opacity,
        node.clip_to_bounds,
        node.children.len(),
        content
    );
}

fn build_paragraph(
    payload: &TextDrawPayload,
    width: f32,
    color: [u8; 4],
    font_signature: u64,
) -> Option<Paragraph> {
    let mut typography = TypographyContext::default();
    let registered_prepared_font =
        font_signature != 0 && register_prepared_fonts(&mut typography, payload);

    let mut paint = Paint::default();
    paint.set_color(color_with_opacity(color, 1.0));

    let mut style = ParagraphStyle::default();
    style
        .set_foreground(&paint)
        .set_font_size(payload.font_size.max(1.0))
        .set_font_weight(map_font_weight(payload.font_weight))
        .set_text_alignment(TextAlignment::Left)
        .set_text_direction(TextDirection::LTR);

    let family = if registered_prepared_font {
        PREPARED_FONT_ALIAS
    } else {
        fallback_font_family(&payload.font_family)
    };
    if !family.is_empty() && !family.contains('\0') {
        style.set_font_family(family);
    }

    let mut builder = ParagraphBuilder::new(&typography)?;
    builder.push_style(&style);
    builder.add_text(&payload.text);
    builder.pop_style();
    builder.build(width.max(1.0))
}

fn register_prepared_fonts(typography: &mut TypographyContext, payload: &TextDrawPayload) -> bool {
    let mut registered_any = false;
    let mut seen = HashSet::new();
    for run in &payload.prepared.glyph_runs {
        let Some(font_data) = run.font_data.downcast_ref::<vello::peniko::FontData>() else {
            continue;
        };
        let font_id = (font_data.data.id(), font_data.index);
        if !seen.insert(font_id) || font_data.data.is_empty() {
            continue;
        }
        let bytes = font_data.data.data().to_vec();
        match typography.register_font(Cow::Owned(bytes), Some(PREPARED_FONT_ALIAS)) {
            Ok(()) => registered_any = true,
            Err(err) => log::debug!("[DIAG-IMPELLER] register prepared font failed: {err}"),
        }
    }
    registered_any
}

fn prepared_font_signature(payload: &TextDrawPayload) -> u64 {
    let mut hasher = DefaultHasher::new();
    let mut any = false;
    for run in &payload.prepared.glyph_runs {
        if let Some(font_data) = run.font_data.downcast_ref::<vello::peniko::FontData>() {
            font_data.data.id().hash(&mut hasher);
            font_data.index.hash(&mut hasher);
            any = true;
        }
    }
    if any {
        hasher.finish()
    } else {
        0
    }
}

fn text_layout_width(payload: &TextDrawPayload, node: &SceneNode) -> f32 {
    if node.width.is_finite() && node.width > 1.0 {
        node.width as f32
    } else if payload.measured_width.is_finite() && payload.measured_width > 1.0 {
        payload.measured_width
    } else {
        1.0
    }
}

fn fallback_font_family(family: &str) -> &str {
    let trimmed = family.trim();
    if !trimmed.is_empty() && !trimmed.contains('\0') {
        trimmed
    } else {
        default_font_family()
    }
}

#[cfg(target_os = "android")]
fn default_font_family() -> &'static str {
    "sans-serif"
}

#[cfg(target_os = "macos")]
fn default_font_family() -> &'static str {
    "Arial"
}

#[cfg(all(not(target_os = "android"), not(target_os = "macos")))]
fn default_font_family() -> &'static str {
    ""
}

fn map_font_weight(weight: u16) -> FontWeight {
    match weight {
        0..=150 => FontWeight::Thin,
        151..=250 => FontWeight::ExtraLight,
        251..=350 => FontWeight::Light,
        351..=450 => FontWeight::Regular,
        451..=550 => FontWeight::Medium,
        551..=650 => FontWeight::SemiBold,
        651..=750 => FontWeight::Bold,
        751..=850 => FontWeight::ExtraBold,
        _ => FontWeight::Black,
    }
}

fn apply_opacity_to_color(mut color: [u8; 4], opacity: f32) -> [u8; 4] {
    color[3] = ((color[3] as f32) * opacity.clamp(0.0, 1.0))
        .round()
        .clamp(0.0, 255.0) as u8;
    color
}

fn color_with_opacity(color: [u8; 4], opacity: f32) -> Color {
    Color::new_srgba(
        color[0] as f32 / 255.0,
        color[1] as f32 / 255.0,
        color[2] as f32 / 255.0,
        (color[3] as f32 / 255.0 * opacity).clamp(0.0, 1.0),
    )
}

fn draw_rect_or_round(builder: &mut DisplayListBuilder, rect: &Rect, radius: f32, paint: &Paint) {
    // TODO: re-enable native rounded-rect once Android Vulkan Impeller output
    // is visually verified. On the current device, `draw_rounded_rect` records
    // commands but the mixed-heavy cards are not visible; using plain rects
    // keeps the Impeller path useful instead of presenting an apparently black
    // root.
    let use_native_round_rect = false;
    if use_native_round_rect && radius > 0.0 {
        let r = radius.max(0.0);
        let radii = RoundingRadii {
            top_left: Point::new(r, r),
            top_right: Point::new(r, r),
            bottom_left: Point::new(r, r),
            bottom_right: Point::new(r, r),
        };
        builder.draw_rounded_rect(rect, &radii, paint);
    } else {
        builder.draw_rect(rect, paint);
    }
}
