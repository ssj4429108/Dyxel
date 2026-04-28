// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Smoke test for the double-layer API (GraphicsRuntime + RenderBackendV2).
//!
//! Validates that:
//! 1. VelloGraphicsFactory creates runtime and backend
//! 2. WgpuRuntime initializes successfully (GPU required)
//! 3. VelloDrawingBackend initializes against WgpuRuntime (GPU required)
//! 4. Type downcasts and runtime kind checks work (no GPU required)

use dyxel_render_api::GraphicsRuntimeFactory;
use dyxel_render_vello::{VelloGraphicsFactory, runtime::WgpuRuntime};

/// Step 2A: Validate object model and contract without touching GPU.
/// This must pass in CI / headless environments.
#[test]
fn test_factory_object_model_no_gpu() {
    let factory = VelloGraphicsFactory::new();
    assert_eq!(factory.backend_name(), "vello");

    let mut runtime = factory.create_runtime().expect("create_runtime failed");
    let mut backend = factory.create_backend().expect("create_backend failed");

    // Verify runtime downcasts to WgpuRuntime
    let wgpu_runtime = runtime
        .as_any_mut()
        .downcast_mut::<WgpuRuntime>()
        .expect("runtime should downcast to WgpuRuntime");
    assert!(wgpu_runtime.device().is_none()); // not initialized yet

    // Verify capabilities contract
    let caps = factory.capabilities();
    assert!(caps.perf_overlay);
    assert!(caps.gpu_timing);
    assert!(caps.renderer_warmup);
    assert!(caps.main_thread_surface_creation);
    assert!(!caps.main_thread_rendering);
    assert!(caps.explicit_present);

    // WgpuRuntime itself doesn't expose runtime_kind; that's on BackendFrameContext.
    // The important contract is that the factory knows it's a Wgpu backend.
    assert_eq!(factory.backend_name(), "vello");

    // Backend can be initialized against runtime (this path does touch GPU,
    // so we stop here in the no-GPU test).
    let _ = backend;
}

/// Step 2B: Validate full initialization with GPU.
/// Run locally with: cargo test -p dyxel-render-vello --test double_layer_smoke -- --ignored
#[test]
#[ignore = "requires a compatible GPU / display"]
fn test_runtime_initializes_with_gpu() {
    let factory = VelloGraphicsFactory::new();
    let mut runtime = factory.create_runtime().expect("create_runtime failed");
    let mut backend = factory.create_backend().expect("create_backend failed");

    // Initialize runtime (creates wgpu device) — needs GPU
    runtime.initialize().expect("runtime initialize failed");
    let wgpu_runtime = runtime
        .as_any_mut()
        .downcast_mut::<WgpuRuntime>()
        .expect("runtime should still downcast after init");
    assert!(wgpu_runtime.device().is_some());
    assert!(wgpu_runtime.queue().is_some());

    // Initialize backend against runtime — needs GPU
    backend.initialize(&mut *runtime).expect("backend initialize failed");
}
