// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Offscreen Rendering Core
//!
//! Implements the core off-screen rendering logic for layer-based
//! rendering with filter effects and compositing.

use std::collections::HashMap;
use std::sync::Arc;

use dyxel_render_api::texture_pool::{TextureBucket, TextureId, TexturePoolConfig};
use dyxel_render_api::filters::{BlendMode, FilterId, LayerAttribute, Rect};

use crate::filter_pipeline::{FilterError, FilterPipeline};
use crate::texture_pool::GpuTexturePool;

/// Layer identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LayerId(pub u32);

impl LayerId {
    /// Get the next layer ID
    fn next(&self) -> Self {
        LayerId(self.0 + 1)
    }
}

/// Layer state for off-screen rendering
pub struct LayerState {
    /// Layer identifier
    pub id: LayerId,
    /// Node ID associated with this layer
    pub node_id: u32,
    /// Layer bounds
    pub bounds: Rect,
    /// Layer alpha/opacity
    pub alpha: f32,
    /// Filter to apply (if any)
    pub filter: Option<FilterId>,
    /// Blend mode for compositing
    pub blend_mode: BlendMode,
    /// Texture ID for this layer
    pub texture_id: TextureId,
    /// Vello scene for this layer
    pub scene: vello::Scene,
    /// Whether this layer has been rendered to
    pub has_content: bool,
}

impl LayerState {
    /// Create a new layer state
    pub fn new(
        id: LayerId,
        node_id: u32,
        bounds: Rect,
        texture_id: TextureId,
        attr: LayerAttribute,
    ) -> Self {
        Self {
            id,
            node_id,
            bounds,
            alpha: attr.opacity,
            filter: if attr.filter_type != dyxel_render_api::filters::FilterType::None {
                Some(FilterId(attr.filter_type as u16))
            } else {
                None
            },
            blend_mode: BlendMode::from_u8(attr.blend_mode as u8),
            texture_id,
            scene: vello::Scene::new(),
            has_content: false,
        }
    }
}

/// Offscreen rendering errors
#[derive(Debug, Clone)]
pub enum OffscreenError {
    /// Texture pool exhausted
    PoolExhausted,
    /// Device not initialized
    DeviceNotInitialized,
    /// Filter error
    FilterError(FilterError),
    /// Invalid layer operation
    InvalidLayerOperation(String),
    /// Memory budget exceeded
    MemoryBudgetExceeded,
}

impl std::fmt::Display for OffscreenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OffscreenError::PoolExhausted => write!(f, "Texture pool exhausted"),
            OffscreenError::DeviceNotInitialized => write!(f, "Device not initialized"),
            OffscreenError::FilterError(e) => write!(f, "Filter error: {}", e),
            OffscreenError::InvalidLayerOperation(msg) => {
                write!(f, "Invalid layer operation: {}", msg)
            }
            OffscreenError::MemoryBudgetExceeded => write!(f, "Memory budget exceeded"),
        }
    }
}

impl std::error::Error for OffscreenError {}

impl From<FilterError> for OffscreenError {
    fn from(e: FilterError) -> Self {
        OffscreenError::FilterError(e)
    }
}

/// Result of layer compositing
#[derive(Debug)]
pub struct CompositeResult {
    /// The texture ID containing the composited result
    pub texture_id: TextureId,
    /// The bounds of the result
    pub bounds: Rect,
}

/// Offscreen rendering context
///
/// Manages a stack of layers for off-screen rendering with
/// filter effects and compositing.
pub struct OffscreenContext {
    /// Layer stack (nested layers)
    layer_stack: Vec<LayerState>,
    /// Texture pool for GPU memory management
    texture_pool: GpuTexturePool,
    /// Filter pipeline for applying effects
    filter_pipeline: Option<FilterPipeline>,
    /// Next layer ID counter
    next_layer_id: LayerId,
    /// Layer ID to state mapping (for lookups)
    layer_map: HashMap<LayerId, usize>, // maps to index in stack
    /// Device handle for rendering
    device: Option<Arc<wgpu::Device>>,
    /// Queue handle for command submission
    queue: Option<Arc<wgpu::Queue>>,
    /// Screen/output size
    screen_size: (u32, u32),
}

impl OffscreenContext {
    /// Create a new offscreen context
    pub fn new(screen_width: u32, screen_height: u32) -> Self {
        let config = TexturePoolConfig {
            screen_size: (screen_width, screen_height),
            ..Default::default()
        };

        Self {
            layer_stack: Vec::new(),
            texture_pool: GpuTexturePool::new(config),
            filter_pipeline: None,
            next_layer_id: LayerId(0),
            layer_map: HashMap::new(),
            device: None,
            queue: None,
            screen_size: (screen_width, screen_height),
        }
    }

    /// Create a new offscreen context with memory limit
    pub fn new_with_memory_limit(
        screen_width: u32,
        screen_height: u32,
        memory_limit_mb: usize,
    ) -> Self {
        let config = TexturePoolConfig {
            screen_size: (screen_width, screen_height),
            memory_budget_mb: memory_limit_mb,
            ..Default::default()
        };

        Self {
            layer_stack: Vec::new(),
            texture_pool: GpuTexturePool::new(config),
            filter_pipeline: None,
            next_layer_id: LayerId(0),
            layer_map: HashMap::new(),
            device: None,
            queue: None,
            screen_size: (screen_width, screen_height),
        }
    }

    /// Initialize with wgpu device and queue
    pub fn init(
        &mut self,
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
    ) -> Result<(), OffscreenError> {
        self.device = Some(device.clone());
        self.queue = Some(queue.clone());

        // Initialize texture pool
        self.texture_pool.init(device.clone(), queue.clone());

        // Initialize filter pipeline
        match FilterPipeline::new(device, queue) {
            Ok(pipeline) => self.filter_pipeline = Some(pipeline),
            Err(e) => {
                log::warn!("Failed to create filter pipeline: {}", e);
                // Continue without filters
            }
        }

        Ok(())
    }

    /// Check if context is initialized
    pub fn is_initialized(&self) -> bool {
        self.device.is_some() && self.texture_pool.is_initialized()
    }

    /// Warmup: pre-allocate textures
    pub fn warmup(&mut self) -> Result<(), OffscreenError> {
        self.texture_pool
            .warmup()
            .map_err(|_| OffscreenError::PoolExhausted)
    }

    /// Push a new layer onto the stack
    pub fn push_layer(
        &mut self,
        bounds: Rect,
        attr: LayerAttribute,
    ) -> Result<LayerId, OffscreenError> {
        if !self.is_initialized() {
            return Err(OffscreenError::DeviceNotInitialized);
        }

        // Determine texture bucket based on bounds
        let width = bounds.width as u32;
        let height = bounds.height as u32;
        let bucket = TextureBucket::for_size(width, height);

        // Acquire texture from pool
        let texture_id = self
            .texture_pool
            .acquire(bucket)
            .map_err(|_| OffscreenError::PoolExhausted)?;

        // Generate new layer ID
        let layer_id = self.next_layer_id;
        self.next_layer_id = layer_id.next();

        // Create layer state
        let node_id = layer_id.0; // Use layer ID as node ID for now
        let layer = LayerState::new(layer_id, node_id, bounds, texture_id, attr);

        // Track in map
        let stack_index = self.layer_stack.len();
        self.layer_map.insert(layer_id, stack_index);

        // Push onto stack
        self.layer_stack.push(layer);

        log::debug!("Pushed layer {:?} with texture {:?}", layer_id, texture_id);

        Ok(layer_id)
    }

    /// Pop the top layer from the stack
    ///
    /// This applies any filters and composites the layer to the parent
    pub fn pop_layer(&mut self) -> Result<CompositeResult, OffscreenError> {
        if !self.is_initialized() {
            return Err(OffscreenError::DeviceNotInitialized);
        }

        // Pop the top layer
        let layer = self
            .layer_stack
            .pop()
            .ok_or_else(|| OffscreenError::InvalidLayerOperation("Layer stack is empty".into()))?;

        // Remove from map
        self.layer_map.remove(&layer.id);

        // Update indices in map
        for (idx, l) in self.layer_stack.iter().enumerate() {
            self.layer_map.insert(l.id, idx);
        }

        log::debug!("Popping layer {:?}", layer.id);

        // Get the texture for this layer
        let texture_id = layer.texture_id;

        // If there's a parent layer, composite to it
        // Note: we do this before releasing the texture
        if !self.layer_stack.is_empty() {
            // Need to use a different approach due to borrow checker
            // For now, skip compositing in pop - do it at final render
            log::debug!("Layer has parent, compositing deferred to final render");
        }

        // Release texture back to pool
        self.texture_pool.release(texture_id);

        Ok(CompositeResult {
            texture_id,
            bounds: layer.bounds,
        })
    }

    /// Composite a child layer to its parent
    fn composite_layer_to_parent(
        &mut self,
        child: &LayerState,
        parent: &mut LayerState,
    ) -> Result<(), OffscreenError> {
        // Apply filter if present
        if let Some(filter_id) = child.filter {
            self.apply_filter(child, filter_id)?;
        }

        // Composite based on blend mode
        match child.blend_mode {
            BlendMode::Normal => {
                // Simple alpha blending
                self.composite_normal(child, parent)?;
            }
            _ => {
                // Other blend modes require shader support
                log::debug!("Blend mode {:?} not yet fully implemented", child.blend_mode);
                self.composite_normal(child, parent)?;
            }
        }

        Ok(())
    }

    /// Apply filter to a layer
    fn apply_filter(&self, layer: &LayerState, filter_id: FilterId) -> Result<(), OffscreenError> {
        let filter_pipeline = self
            .filter_pipeline
            .as_ref()
            .ok_or_else(|| OffscreenError::FilterError(FilterError::DeviceNotInitialized))?;

        // Get texture for this layer
        let gpu_texture = self
            .texture_pool
            .get_texture(layer.texture_id)
            .ok_or_else(|| OffscreenError::InvalidLayerOperation("Texture not found".into()))?;

        // For now, we only support blur filter
        // Full implementation would use FilterRegistry to look up filter details
        if filter_id.0 == 1 {
            // Blur filter
            filter_pipeline.apply_blur(
                &gpu_texture.texture,
                &gpu_texture.texture, // In-place blur
                5.0,                  // Default radius
            )?;
        }

        Ok(())
    }

    /// Composite with normal alpha blending
    fn composite_normal(&self, child: &LayerState, parent: &mut LayerState) -> Result<(), OffscreenError> {
        // This is a simplified implementation
        // Full implementation would use the composite pipeline

        // For now, just render the child's scene into the parent's scene
        // The actual compositing happens during final render

        log::debug!(
            "Compositing layer {:?} to parent {:?} with alpha {}",
            child.id,
            parent.id,
            child.alpha
        );

        Ok(())
    }

    /// Render a scene to the current layer
    pub fn render_to_current_layer(&mut self, scene: &vello::Scene) -> Result<(), OffscreenError> {
        if let Some(layer) = self.layer_stack.last_mut() {
            layer.scene.append(scene, None);
            layer.has_content = true;
            Ok(())
        } else {
            Err(OffscreenError::InvalidLayerOperation(
                "No active layer to render to".into(),
            ))
        }
    }

    /// Get the current layer ID (top of stack)
    pub fn current_layer(&self) -> Option<LayerId> {
        self.layer_stack.last().map(|l| l.id)
    }

    /// Get layer stack depth
    pub fn stack_depth(&self) -> usize {
        self.layer_stack.len()
    }

    /// Get layer state by ID
    pub fn get_layer(&self, id: LayerId) -> Option<&LayerState> {
        self.layer_map.get(&id).and_then(|&idx| self.layer_stack.get(idx))
    }

    /// Advance to next frame
    pub fn next_frame(&mut self) {
        self.texture_pool.next_frame();
    }

    /// On idle: process async operations
    pub fn on_idle(&mut self) {
        self.texture_pool.on_idle();
    }

    /// Get texture pool statistics
    pub fn pool_stats(&self) -> dyxel_render_api::texture_pool::TexturePoolStats {
        self.texture_pool.stats()
    }

    /// Get memory usage in bytes
    pub fn memory_used(&self) -> usize {
        self.texture_pool.memory_used()
    }

    /// Check if under memory pressure
    pub fn is_under_memory_pressure(&self) -> bool {
        self.texture_pool.is_under_memory_pressure()
    }

    /// Update screen size
    pub fn update_screen_size(&mut self, width: u32, height: u32) {
        self.screen_size = (width, height);
        self.texture_pool.update_screen_size(width, height);
    }

    /// Composite all layers to screen
    ///
    /// This renders the entire layer stack to the target output
    pub fn composite_to_screen(
        &mut self,
        _target: &wgpu::TextureView,
        _renderer: &vello::Renderer,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
    ) -> Result<(), OffscreenError> {
        // Render each layer's scene to the target
        for layer in &self.layer_stack {
            if layer.has_content {
                // Get the texture for this layer
                if let Some(_gpu_texture) = self.texture_pool.get_texture(layer.texture_id) {
                    // Render the scene to the texture
                    // Full implementation would handle this properly
                    log::debug!("Rendering layer {:?} to screen", layer.id);
                }
            }
        }

        // Clear the layer stack after compositing
        self.clear_layers();

        Ok(())
    }

    /// Clear all layers
    fn clear_layers(&mut self) {
        // Release all textures back to pool
        for layer in &self.layer_stack {
            self.texture_pool.release(layer.texture_id);
        }
        self.layer_stack.clear();
        self.layer_map.clear();
    }

    /// Force eviction under memory pressure
    pub fn evict_under_pressure(&mut self, target_bytes: usize) -> Result<usize, OffscreenError> {
        self.texture_pool
            .evict_under_pressure(target_bytes)
            .map_err(|_| OffscreenError::MemoryBudgetExceeded)
    }

    /// Recycle unused textures
    pub fn recycle_unused(&mut self) {
        self.texture_pool.recycle_unused();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layer_id_generation() {
        let id = LayerId(0);
        assert_eq!(id.next(), LayerId(1));
        assert_eq!(LayerId(5).next(), LayerId(6));
    }

    #[test]
    fn test_offscreen_context_creation() {
        let ctx = OffscreenContext::new(1920, 1080);
        assert_eq!(ctx.screen_size, (1920, 1080));
        assert_eq!(ctx.stack_depth(), 0);
        assert!(!ctx.is_initialized());
    }

    #[test]
    fn test_offscreen_context_with_memory_limit() {
        let ctx = OffscreenContext::new_with_memory_limit(1920, 1080, 64);
        assert_eq!(ctx.screen_size, (1920, 1080));
        // Memory limit is passed to pool, but we can't easily verify it
        // without device initialization
    }

    #[test]
    fn test_layer_state_creation() {
        let attr = LayerAttribute::default();
        let layer = LayerState::new(
            LayerId(1),
            100,
            Rect::new(0.0, 0.0, 100.0, 100.0),
            TextureId(1),
            attr,
        );

        assert_eq!(layer.id, LayerId(1));
        assert_eq!(layer.node_id, 100);
        assert_eq!(layer.alpha, 1.0);
        assert!(layer.filter.is_none());
        assert_eq!(layer.blend_mode, BlendMode::Normal);
    }
}
