// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Per-backend frame resources used by the legacy direct Vello render path.

use vello::wgpu;

/// A single slot in the triple-buffer ring.
pub(crate) struct TripleBufferSlot {
    pub(crate) texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    pub(crate) bind_group: wgpu::BindGroup,
}

/// Triple-buffered offscreen textures.
///
/// We rotate through 3 independent textures so that the GPU can still be
/// reading from frame N (final blit / present) while the CPU records
/// frame N+1 into a different texture. This eliminates the resource
/// contention that manifests as occasional JANK in Immediate mode.
pub(crate) struct TripleBuffer {
    pub(crate) slots: [TripleBufferSlot; 3],
    pub(crate) current_index: usize,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl TripleBuffer {
    /// Advance the ring index.
    pub(crate) fn advance(&mut self) {
        self.current_index = (self.current_index + 1) % 3;
    }

    /// Return the currently-active slot.
    pub(crate) fn current(&self) -> &TripleBufferSlot {
        &self.slots[self.current_index]
    }
}
