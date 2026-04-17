// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Pointer Event Router
//!
//! Routes raw pointer events to registered callbacks.

use crate::events::PointerEvent;
use std::collections::HashMap;

pub type PointerRouteCallback = Box<dyn FnMut(&PointerEvent) + Send>;

pub struct PointerRouter {
    routes: HashMap<u32, Vec<PointerRouteCallback>>,
    global_routes: Vec<PointerRouteCallback>,
}

impl PointerRouter {
    pub fn new() -> Self {
        Self {
            routes: HashMap::new(),
            global_routes: Vec::new(),
        }
    }

    pub fn add_route(&mut self, pointer: u32, callback: PointerRouteCallback) {
        self.routes
            .entry(pointer)
            .or_insert_with(Vec::new)
            .push(callback);
    }

    pub fn remove_route(&mut self, pointer: u32) {
        self.routes.remove(&pointer);
    }

    pub fn add_global_route(&mut self, callback: PointerRouteCallback) {
        self.global_routes.push(callback);
    }

    pub fn route(&mut self, event: &PointerEvent) {
        // Global routes first
        for callback in &mut self.global_routes {
            callback(event);
        }

        // Pointer-specific routes
        if let Some(callbacks) = self.routes.get_mut(&event.pointer_id) {
            for callback in callbacks {
                callback(event);
            }
        }
    }
}
