// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Raster Cache - Automatic Vector to Bitmap Caching
//!
//! Monitors node stability and automatically bakes static content to textures,
//! reducing render complexity from O(Paths) to O(1) for cached nodes.

use std::collections::HashMap;

/// Opaque texture identifier used by the raster cache.
/// Concrete backends map this to their own GPU texture handles.
#[derive(Debug)]
pub struct TextureId(pub u32);

impl Clone for TextureId {
    fn clone(&self) -> Self {
        TextureId(self.0)
    }
}

impl Copy for TextureId {}

/// Frame count before a node is considered stable enough to bake
const STABLE_FRAME_THRESHOLD: u32 = 30;

/// Frame count after which unused cached textures can be recycled
const UNUSED_FRAME_THRESHOLD: u64 = 60;

/// Raster cache configuration
#[derive(Debug, Clone)]
pub struct RasterCacheConfig {
    /// Frames of stability before baking (default: 30)
    pub stable_frame_threshold: u32,
    /// Enable automatic baking
    pub auto_bake: bool,
    /// Maximum memory budget for cached textures (MB)
    pub memory_budget_mb: usize,
    /// Minimum node complexity to consider for caching
    /// (nodes with fewer paths than this won't be cached)
    pub min_path_count: usize,
}

impl Default for RasterCacheConfig {
    fn default() -> Self {
        Self {
            stable_frame_threshold: STABLE_FRAME_THRESHOLD,
            auto_bake: true,
            memory_budget_mb: 64, // 64MB for raster cache
            min_path_count: 10,
        }
    }
}

/// Cache entry state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheEntryState {
    /// Node is being tracked but not yet stable
    Tracking,
    /// Node is stable and ready to be baked
    ReadyToBake,
    /// Node is baked to texture
    Baked,
    /// Baking in progress
    Baking,
}

/// Raster cache entry for a node
#[derive(Debug)]
pub struct CacheEntry {
    /// Node ID
    pub node_id: u32,
    /// Current cache state
    pub state: CacheEntryState,
    /// Consecutive stable frames
    pub stable_frames: u32,
    /// Frame when last baked
    pub last_bake_frame: u64,
    /// Frame when last accessed
    pub last_access_frame: u64,
    /// Cached texture ID
    pub texture_id: Option<TextureId>,
    /// Texture size when baked
    pub texture_size: (u32, u32),
    /// Estimated path count (for complexity tracking)
    pub path_count: usize,
    /// Cache hits count
    pub hit_count: u64,
}

impl CacheEntry {
    /// Create a new cache entry
    pub fn new(node_id: u32) -> Self {
        Self {
            node_id,
            state: CacheEntryState::Tracking,
            stable_frames: 0,
            last_bake_frame: 0,
            last_access_frame: 0,
            texture_id: None,
            texture_size: (0, 0),
            path_count: 0,
            hit_count: 0,
        }
    }

    /// Check if entry is ready to be baked
    pub fn is_ready_to_bake(&self, threshold: u32) -> bool {
        self.state == CacheEntryState::Tracking && self.stable_frames >= threshold
    }

    /// Record a cache hit
    pub fn record_hit(&mut self, current_frame: u64) {
        self.hit_count += 1;
        self.last_access_frame = current_frame;
    }

    /// Mark as baked
    pub fn mark_baked(&mut self, texture_id: TextureId, size: (u32, u32), frame: u64) {
        self.state = CacheEntryState::Baked;
        self.texture_id = Some(texture_id);
        self.texture_size = size;
        self.last_bake_frame = frame;
        self.last_access_frame = frame;
    }

    /// Check if this entry can be recycled (unused for too long)
    pub fn can_recycle(&self, current_frame: u64, threshold: u64) -> bool {
        self.state == CacheEntryState::Baked
            && (current_frame - self.last_access_frame) > threshold
    }
}

/// Statistics for raster cache performance
#[derive(Debug, Clone, Default)]
pub struct RasterCacheStats {
    /// Total tracked nodes
    pub tracked_nodes: usize,
    /// Currently baked nodes
    pub baked_nodes: usize,
    /// Total cache hits
    pub total_hits: u64,
    /// Total cache misses
    pub total_misses: u64,
    /// Memory used by cached textures (bytes)
    pub memory_used: usize,
    /// Number of automatic bakes performed
    pub auto_bake_count: u64,
    /// Number of texture evictions
    pub eviction_count: u64,
}

/// Raster cache manager
pub struct RasterCache {
    /// Cache configuration
    config: RasterCacheConfig,
    /// Cache entries by node ID
    entries: HashMap<u32, CacheEntry>,
    /// Current frame number
    current_frame: u64,
    /// Statistics
    stats: RasterCacheStats,
}

impl RasterCache {
    /// Create a new raster cache
    pub fn new(config: RasterCacheConfig) -> Self {
        Self {
            config,
            entries: HashMap::new(),
            current_frame: 0,
            stats: RasterCacheStats::default(),
        }
    }

    /// Advance to next frame
    pub fn next_frame(&mut self) {
        self.current_frame += 1;
    }

    /// Track a node for potential caching
    pub fn track_node(&mut self, node_id: u32, path_count: usize) {
        if path_count < self.config.min_path_count {
            return; // Too simple to cache
        }

        self.entries
            .entry(node_id)
            .or_insert_with(|| {
                self.stats.tracked_nodes += 1;
                CacheEntry::new(node_id)
            })
            .path_count = path_count;
    }

    /// Mark a node as dirty (content changed)
    pub fn mark_dirty(&mut self, node_id: u32) {
        if let Some(entry) = self.entries.get_mut(&node_id) {
            // Always reset stability tracking when dirty
            entry.stable_frames = 0;
            if entry.state == CacheEntryState::Baked || entry.state == CacheEntryState::ReadyToBake {
                entry.state = CacheEntryState::Tracking;
                self.stats.baked_nodes = self.stats.baked_nodes.saturating_sub(1);
            }
        }
    }

    /// Check if a node has a valid cached texture
    pub fn get_cached_texture(&mut self, node_id: u32) -> Option<TextureId> {
        if let Some(entry) = self.entries.get_mut(&node_id) {
            if entry.state == CacheEntryState::Baked {
                entry.record_hit(self.current_frame);
                self.stats.total_hits += 1;
                return entry.texture_id;
            }
        }
        self.stats.total_misses += 1;
        None
    }

    /// Update stability for all tracked nodes
    /// Returns list of nodes ready to be baked
    pub fn update_stability(&mut self, dirty_tracker: &crate::dirty::DirtyTracker) -> Vec<u32> {
        let mut ready_to_bake = Vec::new();

        for (node_id, entry) in &mut self.entries {
            if entry.state == CacheEntryState::Tracking || entry.state == CacheEntryState::Baked {
                if dirty_tracker.is_node_dirty(*node_id) {
                    // Node is dirty, reset stability
                    entry.stable_frames = 0;
                    if entry.state == CacheEntryState::Baked {
                        entry.state = CacheEntryState::Tracking;
                        self.stats.baked_nodes = self.stats.baked_nodes.saturating_sub(1);
                    }
                } else {
                    // Node is stable
                    entry.stable_frames += 1;

                    if entry.is_ready_to_bake(self.config.stable_frame_threshold) {
                        entry.state = CacheEntryState::ReadyToBake;
                        ready_to_bake.push(*node_id);
                    }
                }
            }
        }

        ready_to_bake
    }

    /// Bake a node to texture
    pub fn bake_node(
        &mut self,
        node_id: u32,
        texture_id: TextureId,
        texture_size: (u32, u32),
    ) -> bool {
        if let Some(entry) = self.entries.get_mut(&node_id) {
            if entry.state == CacheEntryState::ReadyToBake {
                entry.mark_baked(texture_id, texture_size, self.current_frame);
                self.stats.baked_nodes += 1;
                self.stats.auto_bake_count += 1;
                self.stats.memory_used += (texture_size.0 * texture_size.1 * 4) as usize;
                return true;
            }
        }
        false
    }

    /// Force immediate bake (for manual cache control)
    pub fn force_bake(&mut self, node_id: u32) -> bool {
        if let Some(entry) = self.entries.get_mut(&node_id) {
            entry.state = CacheEntryState::ReadyToBake;
            entry.stable_frames = self.config.stable_frame_threshold;
            true
        } else {
            false
        }
    }

    /// Invalidate and remove a cached node
    pub fn invalidate(&mut self, node_id: u32) -> Option<TextureId> {
        if let Some(entry) = self.entries.remove(&node_id) {
            self.stats.tracked_nodes = self.stats.tracked_nodes.saturating_sub(1);
            if entry.state == CacheEntryState::Baked {
                self.stats.baked_nodes = self.stats.baked_nodes.saturating_sub(1);
                self.stats.memory_used = self.stats
                    .memory_used
                    .saturating_sub((entry.texture_size.0 * entry.texture_size.1 * 4) as usize);
                return entry.texture_id;
            }
        }
        None
    }

    /// Recycle unused cached textures.
    /// Returns (node_id, texture_id) pairs that should be released by the backend.
    pub fn recycle_unused(&mut self) -> Vec<(u32, TextureId)> {
        let to_recycle: Vec<u32> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.can_recycle(self.current_frame, UNUSED_FRAME_THRESHOLD))
            .map(|(id, _)| *id)
            .collect();

        let mut recycled = Vec::new();
        for node_id in to_recycle {
            if let Some(texture_id) = self.invalidate(node_id) {
                recycled.push((node_id, texture_id));
                self.stats.eviction_count += 1;
            }
        }

        recycled
    }

    /// Check if we need to evict textures due to memory pressure.
    /// Returns (node_id, texture_id) pairs that should be released by the backend.
    pub fn check_memory_pressure(&mut self) -> Vec<(u32, TextureId)> {
        let budget_bytes = self.config.memory_budget_mb * 1024 * 1024;

        if self.stats.memory_used <= budget_bytes {
            return Vec::new();
        }

        // Evict least recently used entries
        let mut entries_by_access: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, e)| e.state == CacheEntryState::Baked)
            .map(|(id, e)| (*id, e.last_access_frame, e.texture_size))
            .collect();

        entries_by_access.sort_by_key(|(_, frame, _)| *frame);

        let mut recycled = Vec::new();
        let mut memory_freed = 0;
        let target_free = self.stats.memory_used - budget_bytes;

        for (node_id, _, texture_size) in entries_by_access {
            if memory_freed >= target_free {
                break;
            }

            let entry_memory = (texture_size.0 * texture_size.1 * 4) as usize;

            if let Some(texture_id) = self.invalidate(node_id) {
                memory_freed += entry_memory;
                recycled.push((node_id, texture_id));
                self.stats.eviction_count += 1;
            }
        }

        recycled
    }

    /// Get cache statistics
    pub fn stats(&self) -> &RasterCacheStats {
        &self.stats
    }

    /// Get hit rate (0.0 to 1.0)
    pub fn hit_rate(&self) -> f64 {
        let total = self.stats.total_hits + self.stats.total_misses;
        if total > 0 {
            self.stats.total_hits as f64 / total as f64
        } else {
            0.0
        }
    }

    /// Clear all cache entries.
    /// Returns the list of texture IDs that should be released by the backend.
    pub fn clear(&mut self) -> Vec<TextureId> {
        let texture_ids: Vec<_> = self
            .entries
            .values()
            .filter_map(|e| e.texture_id)
            .collect();

        self.entries.clear();
        self.stats = RasterCacheStats::default();

        texture_ids
    }

    /// Get list of all baked texture IDs
    pub fn get_all_baked_textures(&self) -> Vec<(u32, TextureId)> {
        self.entries
            .values()
            .filter(|e| e.state == CacheEntryState::Baked)
            .filter_map(|e| e.texture_id.map(|tid| (e.node_id, tid)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_entry_stability() {
        let mut entry = CacheEntry::new(1);
        assert!(!entry.is_ready_to_bake(30));

        entry.stable_frames = 30;
        assert!(entry.is_ready_to_bake(30));
    }

    #[test]
    fn test_cache_hit_tracking() {
        let mut entry = CacheEntry::new(1);
        entry.record_hit(100);
        assert_eq!(entry.hit_count, 1);
        assert_eq!(entry.last_access_frame, 100);

        entry.record_hit(200);
        assert_eq!(entry.hit_count, 2);
        assert_eq!(entry.last_access_frame, 200);
    }

    #[test]
    fn test_hit_rate() {
        let mut cache = RasterCache::new(RasterCacheConfig::default());

        // Simulate some hits and misses
        cache.stats.total_hits = 90;
        cache.stats.total_misses = 10;

        assert!((cache.hit_rate() - 0.9).abs() < 0.001);
    }

    #[test]
    fn test_mark_dirty_resets_stability() {
        let mut cache = RasterCache::new(RasterCacheConfig::default());

        cache.track_node(1, 20);

        // Simulate stability
        for _ in 0..30 {
            cache.next_frame();
        }

        // Manually set stable frames
        if let Some(entry) = cache.entries.get_mut(&1) {
            entry.stable_frames = 30;
        }

        // Mark as dirty
        cache.mark_dirty(1);

        if let Some(entry) = cache.entries.get(&1) {
            assert_eq!(entry.stable_frames, 0);
            assert_eq!(entry.state, CacheEntryState::Tracking);
        }
    }
}
