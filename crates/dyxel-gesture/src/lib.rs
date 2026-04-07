// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! # Dyxel Gesture System
//!
//! A robust gesture recognition system strictly replicated from Flutter's GestureArena architecture.
//!
//! ## Architecture
//!
//! Raw Touch Events → PointerRouter → GestureArenaManager → GestureRecognizers → GestureEvents → WASM
//!
//! ## Key Components
//!
//! - **PointerRouter**: Routes raw events to interested recognizers.
//! - **GestureArenaManager**: Orchestrates the competition between recognizers for each pointer.
//! - **GestureArenaMember**: A trait for objects that participate in the arena competition.
//! - **GestureRecognizer**: Base trait for high-level gesture recognition (Tap, Pan, LongPress, etc.).

// Core modules
pub mod arena;
pub mod recognizer;
pub mod router;
pub mod router_integration;

// Shared types
mod events;
mod hit_test;
mod spatial_hit_tester;

// Gesture composition for RSX DSL
pub mod gesture_composition;

// Test utilities (available with #[cfg(test)])
#[cfg(test)]
pub mod test_utils;

// Re-export core APIs
pub use arena::{
    GestureArenaManager,
    GestureArenaMember,
};

pub use router::PointerRouter;

pub use recognizer::{
    GestureDisposition,
    RecognizerState,
    GestureRecognizer,
    TapGestureRecognizer,
    PanGestureRecognizer,
    LongPressGestureRecognizer,
    ScaleGestureRecognizer,
    RotationGestureRecognizer,
    GestureCategoryType,
    PanDirection,
};

// Re-export shared types
pub use events::{GestureEvent, GestureEventType, PointerEvent, PointerData, PointerEventType};
pub use events::{PanPhase, LongPressPhase, GesturePhase};
pub use hit_test::{HitTestResult, HitTester, NoOpHitTester, RectHitTester, LayoutHitTester};
pub use spatial_hit_tester::{SpatialHitTester, SpatialStats};

// Router integration for dyxel-core
pub use router_integration::{GestureRouter, GestureConfig, GestureType};

// Gesture composition for RSX DSL
pub use gesture_composition::{
    ExclusiveGesture, SimultaneousGesture, SequencedGesture,
    GestureRelationship, ComposableGesture,
};

/// Global gesture configuration
///
/// Values are aligned with Flutter's gesture constants:
/// https://api.flutter.dev/flutter/gestures/gestures-library.html
#[derive(Debug, Clone, Copy)]
pub struct GestureSettings {
    /// Maximum duration for a tap (milliseconds)
    pub tap_timeout_ms: u64,
    /// Maximum movement for a tap (logical pixels)
    pub tap_slop: f32,
    /// Duration for long press (milliseconds)
    pub long_press_timeout_ms: u64,
    /// Maximum movement for long press (logical pixels)
    pub long_press_slop: f32,
    /// Minimum movement to start pan (logical pixels)
    pub pan_slop: f32,
    /// Double tap timeout (milliseconds)
    pub double_tap_timeout_ms: u64,
    /// Maximum distance between taps for double tap (logical pixels)
    pub double_tap_slop: f32,
}

impl Default for GestureSettings {
    fn default() -> Self {
        Self {
            tap_timeout_ms: 300,
            tap_slop: 18.0,
            long_press_timeout_ms: 500,
            long_press_slop: 18.0,
            pan_slop: 18.0,
            double_tap_timeout_ms: 300,
            double_tap_slop: 100.0,
        }
    }
}
