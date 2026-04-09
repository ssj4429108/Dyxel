// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Web platform system info provider using Web APIs

use crate::SystemInfoProvider;
use wasm_bindgen::prelude::*;
use web_sys::{window, Navigator, Performance};

pub struct WebSystemInfo;

impl SystemInfoProvider for WebSystemInfo {
    fn get_memory_usage(&self) -> Option<(u64, Option<u64>)> {
        if let Some(window) = window() {
            if let Ok(memory) = window.performance()?.memory() {
                let used = memory.used_js_heap_size() as u64;
                let total = memory.total_js_heap_size() as u64;
                let limit = memory.js_heap_size_limit() as u64;

                // used is in bytes, available is limit - used
                let available = Some(limit);
                return Some((used, available));
            }
        }
        None
    }

    fn get_cpu_usage(&self) -> Option<f32> {
        // Web platform doesn't provide CPU usage directly
        // We could use performance.now() to measure frame time but that's
        // already tracked by the FPS monitor
        None
    }

    fn platform_name(&self) -> &'static str {
        "web"
    }
}

/// Get device memory information (in GB) if available
pub fn get_device_memory_gb() -> Option<f32> {
    if let Some(window) = window() {
        if let Ok(navigator) = window.navigator().dyn_into::<Navigator>() {
            // deviceMemory is available in Chrome
            if let Ok(memory) = js_sys::Reflect::get(&navigator, &JsValue::from_str("deviceMemory"))
            {
                if !memory.is_undefined() && !memory.is_null() {
                    return memory.as_f64().map(|v| v as f32);
                }
            }
        }
    }
    None
}

/// Get hardware concurrency (number of logical processors)
pub fn get_hardware_concurrency() -> Option<u32> {
    if let Some(window) = window() {
        if let Ok(navigator) = window.navigator().dyn_into::<Navigator>() {
            return Some(navigator.hardware_concurrency());
        }
    }
    None
}
