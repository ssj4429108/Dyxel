// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Platform-specific system info providers

use crate::SystemInfoProvider;

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_arch = "wasm32")]
mod web;

/// Create the appropriate system info provider for the current platform
pub fn create_system_info_provider() -> std::sync::Arc<dyn SystemInfoProvider> {
    #[cfg(target_os = "android")]
    {
        std::sync::Arc::new(android::AndroidSystemInfo)
    }
    #[cfg(target_os = "macos")]
    {
        std::sync::Arc::new(macos::MacSystemInfo)
    }
    #[cfg(target_arch = "wasm32")]
    {
        std::sync::Arc::new(web::WebSystemInfo)
    }
    #[cfg(not(any(target_os = "android", target_os = "macos", target_arch = "wasm32")))]
    {
        std::sync::Arc::new(crate::NoopSystemInfoProvider)
    }
}
