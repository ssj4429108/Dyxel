// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! UI Components
//!
//! High-level UI components following the Azure Meridian Design System.

pub mod button;
pub mod flex;
pub mod text_input;

// Re-export flex components
pub use flex::{
    Color, Column, CrossAxisAlignment, Divider, MainAxisAlignment, Padding, Row, Spacer,
};

// Re-export button components
pub use button::{
    ghost_button, outline_button, primary_button, secondary_button, Button, ButtonSize,
    ButtonVariant,
};

// Re-export text_input components
pub use text_input::TextInput;
