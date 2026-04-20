// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Dirty tracking types used by the rendering pipeline.
//!
//! These types are kept in the render API crate so that `RenderPackage`
//! can carry a dirty snapshot without forcing backends to depend on
//! `dyxel-shared`.

/// Node dirty field bitflags for tracking what changed.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirtyField {
    None = 0,
    Position = 1 << 0, // x, y changed
    Size = 1 << 1,     // width, height changed
    Style = 1 << 2,    // color, border, etc.
    Text = 1 << 3,     // text content changed
    Children = 1 << 4, // child add/remove
    Layout = 1 << 5,   // flex properties changed
}

impl DirtyField {
    pub fn from_bits(bits: u8) -> Self {
        Self::from_bits_truncate(bits)
    }

    pub fn from_bits_truncate(bits: u8) -> Self {
        match bits {
            0 => Self::None,
            1 => Self::Position,
            2 => Self::Size,
            4 => Self::Style,
            8 => Self::Text,
            16 => Self::Children,
            32 => Self::Layout,
            _ => Self::None,
        }
    }

    pub fn bits(&self) -> u8 {
        *self as u8
    }

    pub fn contains(&self, other: Self) -> bool {
        self.bits() & other.bits() != 0
    }
}

/// Dirty region tracker using bitset.
#[derive(Debug, Default, Clone)]
pub struct DirtyTracker {
    /// Bitset: 1 bit per node, 1024 nodes = 32 u32 words.
    pub node_bitset: [u32; 32],
    /// Track which fields changed per node (for selective re-render).
    pub node_dirty_fields: std::collections::HashMap<u32, u8>,
    /// Global dirty flag — any change occurred.
    pub any_dirty: bool,
}

impl DirtyTracker {
    pub fn new() -> Self {
        Self {
            node_bitset: [0; 32],
            node_dirty_fields: std::collections::HashMap::new(),
            any_dirty: false,
        }
    }

    /// Mark a node as dirty.
    pub fn mark_dirty(&mut self, node_id: u32, fields: DirtyField) {
        if node_id as usize >= 1024 {
            return;
        }

        let word_idx = (node_id / 32) as usize;
        let bit_idx = node_id % 32;
        self.node_bitset[word_idx] |= 1 << bit_idx;

        let field_bits = fields.bits();
        self.node_dirty_fields
            .entry(node_id)
            .and_modify(|f| *f |= field_bits)
            .or_insert(field_bits);

        self.any_dirty = true;
    }

    /// Check if a node is dirty.
    pub fn is_node_dirty(&self, node_id: u32) -> bool {
        if node_id as usize >= 1024 {
            return false;
        }
        let word_idx = (node_id / 32) as usize;
        let bit_idx = node_id % 32;
        (self.node_bitset[word_idx] >> bit_idx) & 1 != 0
    }

    /// Check if any nodes are dirty.
    pub fn has_dirty(&self) -> bool {
        self.any_dirty
    }

    /// Clear all dirty flags.
    pub fn clear(&mut self) {
        self.node_bitset = [0; 32];
        self.node_dirty_fields.clear();
        self.any_dirty = false;
    }

    /// Iterate over all dirty node IDs.
    pub fn iter_dirty_nodes(&self) -> impl Iterator<Item = u32> + '_ {
        self.node_bitset
            .iter()
            .enumerate()
            .flat_map(|(word_idx, &word)| {
                let mut nodes = Vec::new();
                let mut w = word;
                while w != 0 {
                    let bit = w.trailing_zeros();
                    nodes.push((word_idx as u32 * 32) + bit);
                    w &= w - 1; // Clear lowest set bit
                }
                nodes.into_iter()
            })
    }
}
