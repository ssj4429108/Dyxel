// Sample modules - uncomment one to build:

// Gesture & State Validation Demo
// mod stress_test;

// mod final_demo;

// Gesture Orchestration Demo (RSX Gesture DSL)
// mod gesture_orchestration;

// State Dynamic Binding Demo
// mod state_binding_demo;

// Gesture API Demo (Tap, DoubleTap, LongPress, Pan with state display)
// mod gesture_demo;

// Layer Effects Demo - Vello Native Layer Rendering
// mod layer_effects_demo;

// Performance Benchmark Samples
// 每次只启用一个，通过 mod + pub use 切换
// mod perf_logic_heavy;
// mod perf_render_heavy;
mod perf_mixed_heavy;

// pub use perf_logic_heavy::*;
// pub use perf_render_heavy::*;
pub use perf_mixed_heavy::*;
