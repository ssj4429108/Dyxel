// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! macOS Context Menu Integration
//!
//! This is a stub implementation that logs menu actions.
//! A full implementation would use macOS's NSMenu API.

use dyxel_core::text_input::{ContextMenuIntegration, ContextMenuItem};

pub struct MacContextMenu;

impl MacContextMenu {
    pub fn new() -> Self {
        Self
    }
}

impl ContextMenuIntegration for MacContextMenu {
    fn show_menu(&self, node_id: u32, items: &[ContextMenuItem], position: Option<(f32, f32)>) {
        log::info!("Context menu for node {} at {:?}:", node_id, position);
        for item in items {
            log::info!("  - {:?} ({})", item, item.label());
        }

        // TODO: Implement actual NSMenu display
        // This would involve:
        // 1. Creating an NSMenu
        // 2. Adding NSMenuItem for each ContextMenuItem
        // 3. Displaying at the specified position
    }

    fn hide_menu(&self, _node_id: u32) {
        log::info!("Hiding context menu");
        // TODO: Dismiss NSMenu
    }
}
