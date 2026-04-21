// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Web platform system info provider using Web APIs

use crate::SystemInfoProvider;
use wasm_bindgen::prelude::*;
use web_sys::{window, Navigator};

pub struct WebSystemInfo;

impl SystemInfoProvider for WebSystemInfo {
    fn get_memory_usage(&self) -> Option<(u64, Option<u64>)> {
        if let Some(window) = window() {
            if let Some(performance) = window.performance() {
                // memory() is a Chrome-specific extension; use Reflect to access it dynamically
                let memory = js_sys::Reflect::get(&performance, &JsValue::from_str("memory")).ok()?;
                if memory.is_undefined() || memory.is_null() {
                    return None;
                }
                let used = js_sys::Reflect::get(&memory, &JsValue::from_str("usedJSHeapSize"))
                    .ok()?
                    .as_f64()? as u64;
                let total = js_sys::Reflect::get(&memory, &JsValue::from_str("totalJSHeapSize"))
                    .ok()?
                    .as_f64()? as u64;
                let limit = js_sys::Reflect::get(&memory, &JsValue::from_str("jsHeapSizeLimit"))
                    .ok()?
                    .as_f64()? as u64;

                // available is limit - used
                let _available = Some(limit);
                return Some((used, Some(total)));
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
            return Some(navigator.hardware_concurrency() as u32);
        }
    }
    None
}
