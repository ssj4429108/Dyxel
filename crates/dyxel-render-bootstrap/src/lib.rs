// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Compile-time backend selection bootstrap.
//!
//! This crate is the **only** place in the dependency graph that chooses the
//! concrete render backend. Platform crates and `dyxel-core` depend on it
//! instead of individual backend crates.
//!
//! Exactly one backend feature (`vello`, `impeller`, `skia`) must be enabled.
//! Enabling zero or more than one is a compile error.

use dyxel_render_api::GraphicsRuntimeFactory;

#[cfg(not(any(feature = "vello", feature = "impeller", feature = "skia")))]
compile_error!(
    "dyxel-render-bootstrap: exactly one backend feature must be enabled. \
     Choose one of: vello, impeller, skia"
);

#[cfg(all(feature = "vello", feature = "impeller"))]
compile_error!("dyxel-render-bootstrap: cannot enable both 'vello' and 'impeller' features");

#[cfg(all(feature = "vello", feature = "skia"))]
compile_error!("dyxel-render-bootstrap: cannot enable both 'vello' and 'skia' features");

#[cfg(all(feature = "impeller", feature = "skia"))]
compile_error!("dyxel-render-bootstrap: cannot enable both 'impeller' and 'skia' features");

#[cfg(feature = "skia")]
compile_error!("dyxel-render-bootstrap: 'skia' backend is not yet implemented");

/// Create the compile-time-selected graphics runtime factory.
pub fn create_graphics_factory() -> Box<dyn GraphicsRuntimeFactory> {
    #[cfg(feature = "vello")]
    {
        Box::new(dyxel_render_vello::VelloGraphicsFactory::new())
    }

    #[cfg(feature = "impeller")]
    {
        Box::new(dyxel_render_impeller::ImpellerGraphicsFactory::new())
    }

    // skia is rejected at compile time (see module-level compile_error above)
}

/// Return the name of the selected backend.
pub fn backend_name() -> &'static str {
    #[cfg(feature = "vello")]
    {
        "vello"
    }

    #[cfg(feature = "impeller")]
    {
        "impeller"
    }

    // skia is rejected at compile time (see module-level compile_error above)
}
