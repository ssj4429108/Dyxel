// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Button Component
//!
//! Provides themed button styles based on Azure Meridian Design System.
//! Supports multiple visual states: normal, pressed, disabled.
//!
//! # Variants
//! - Primary: Main action button (blue background)
//! - Secondary: Alternative action (light blue background)
//! - Outline: Subtle action with border
//! - Ghost: Minimal action, text only
//!
//! # Example
//! ```rust,ignore
//! Button::new("Click me")
//!     .variant(ButtonVariant::Primary)
//!     .on_tap(|| { /* action */ })
//! ```

use crate::components::Padding;
use crate::{
    select_node, BaseView, CrossAxisAlignment, GestureEvent, MainAxisAlignment, Row, SizeUnit,
    TapHandlerEntry, Text, View, POINTER_DOWN_HANDLERS, POINTER_UP_HANDLERS, SHARED_BUFFER,
    TAP_HANDLERS,
};
use dyxel_shared::push_command;
use std::cell::RefCell;
use std::collections::HashMap;

/// Button style variants
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonVariant {
    /// Primary action button - blue background
    Primary,
    /// Secondary action button - light blue background
    Secondary,
    /// Outlined button - border only
    Outline,
    /// Ghost button - text only
    Ghost,
    /// Disabled state
    Disabled,
}

impl Default for ButtonVariant {
    fn default() -> Self {
        ButtonVariant::Primary
    }
}

/// Button size variants
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonSize {
    /// Small button
    Small,
    /// Medium (default) button
    Medium,
    /// Large button
    Large,
}

impl Default for ButtonSize {
    fn default() -> Self {
        ButtonSize::Medium
    }
}

/// Button visual state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonState {
    /// Normal state
    Normal,
    /// Pressed/highlighted state
    Pressed,
    /// Disabled state
    Disabled,
}

impl Default for ButtonState {
    fn default() -> Self {
        ButtonState::Normal
    }
}

/// Style configuration for a button variant
#[derive(Debug, Clone, Copy)]
struct ButtonStyle {
    /// Normal state: (background, text_color)
    normal: ((u32, u32, u32, u32), (u8, u8, u8, u8)),
    /// Pressed state: (background, text_color)
    pressed: ((u32, u32, u32, u32), (u8, u8, u8, u8)),
    /// Disabled state: (background, text_color)
    disabled: ((u32, u32, u32, u32), (u8, u8, u8, u8)),
}

impl ButtonStyle {
    /// Get style for a specific state
    fn for_state(&self, state: ButtonState) -> ((u32, u32, u32, u32), (u8, u8, u8, u8)) {
        match state {
            ButtonState::Normal => self.normal,
            ButtonState::Pressed => self.pressed,
            ButtonState::Disabled => self.disabled,
        }
    }
}

/// Predefined styles for each variant
fn get_variant_style(variant: ButtonVariant) -> ButtonStyle {
    match variant {
        ButtonVariant::Primary => ButtonStyle {
            normal: ((0u32, 88, 188, 255), (255u8, 255, 255, 255)), // #0058bc bg, white text
            pressed: ((0u32, 70, 150, 255), (255u8, 255, 255, 255)), // Darker blue, white text
            disabled: ((225u32, 226, 237, 255), (113u8, 119, 134, 255)), // Gray bg, gray text
        },
        ButtonVariant::Secondary => ButtonStyle {
            normal: ((116u32, 209, 255, 255), (0u8, 77, 103, 255)), // #74d1ff bg, dark text
            pressed: ((90u32, 180, 230, 255), (0u8, 60, 85, 255)),  // Darker cyan, dark text
            disabled: ((225u32, 226, 237, 255), (113u8, 119, 134, 255)), // Gray bg, gray text
        },
        ButtonVariant::Outline => ButtonStyle {
            normal: ((255u32, 255, 255, 255), (0u8, 88, 188, 255)), // White bg, blue text
            pressed: ((0u32, 88, 188, 26), (0u8, 88, 188, 255)), // Light blue tint (10% alpha), blue text
            disabled: ((255u32, 255, 255, 255), (113u8, 119, 134, 255)), // White bg, gray text
        },
        ButtonVariant::Ghost => ButtonStyle {
            normal: ((255u32, 255, 255, 0), (0u8, 88, 188, 255)), // Transparent bg, blue text
            pressed: ((0u32, 88, 188, 26), (0u8, 88, 188, 255)), // Light blue tint (10% alpha), blue text
            disabled: ((255u32, 255, 255, 0), (113u8, 119, 134, 255)), // Transparent bg, gray text
        },
        ButtonVariant::Disabled => ButtonStyle {
            normal: ((225u32, 226, 237, 255), (113u8, 119, 134, 255)), // Gray bg, gray text
            pressed: ((225u32, 226, 237, 255), (113u8, 119, 134, 255)), // Same as normal
            disabled: ((225u32, 226, 237, 255), (113u8, 119, 134, 255)), // Same as normal
        },
    }
}

// Global button state tracking (node_id -> state)
thread_local! {
    static BUTTON_STATES: RefCell<HashMap<u32, ButtonState>> = RefCell::new(HashMap::new());
    static BUTTON_DISABLED: RefCell<HashMap<u32, bool>> = RefCell::new(HashMap::new());
}

/// Apply pressed state with color shift (called by pointer down handler)
fn apply_button_pressed_state(node_id: u32, variant: ButtonVariant, _size: ButtonSize) {
    // Check if button is disabled - don't apply pressed state if disabled
    let is_disabled = BUTTON_DISABLED.with(|d| d.borrow().get(&node_id).copied().unwrap_or(false));
    if is_disabled {
        return;
    }

    BUTTON_STATES.with(|states| {
        states.borrow_mut().insert(node_id, ButtonState::Pressed);
    });

    // Get pressed state colors
    let style = get_variant_style(variant);
    let ((bg_r, bg_g, bg_b, bg_a), _) = style.for_state(ButtonState::Pressed);

    // Apply color change
    select_node(node_id);
    push_command!(
        SHARED_BUFFER,
        SetColorCompact,
        bg_r as u8,
        bg_g as u8,
        bg_b as u8,
        bg_a as u8
    );
}

/// Apply normal state (called by pointer up handler)
fn apply_button_normal_state(node_id: u32, variant: ButtonVariant, _size: ButtonSize) {
    // Check if button is disabled - don't change state if disabled
    let is_disabled = BUTTON_DISABLED.with(|d| d.borrow().get(&node_id).copied().unwrap_or(false));
    if is_disabled {
        return;
    }

    BUTTON_STATES.with(|states| {
        states.borrow_mut().insert(node_id, ButtonState::Normal);
    });

    // Get normal state colors
    let style = get_variant_style(variant);
    let ((bg_r, bg_g, bg_b, bg_a), _) = style.for_state(ButtonState::Normal);

    // Apply color change
    select_node(node_id);
    push_command!(
        SHARED_BUFFER,
        SetColorCompact,
        bg_r as u8,
        bg_g as u8,
        bg_b as u8,
        bg_a as u8
    );
}

/// Get current button state
fn get_button_state(node_id: u32) -> ButtonState {
    BUTTON_STATES.with(|states| {
        states
            .borrow()
            .get(&node_id)
            .copied()
            .unwrap_or(ButtonState::Normal)
    })
}

/// A themed button component with multi-state support
pub struct Button {
    view: View,
    label: String,
    variant: ButtonVariant,
    size: ButtonSize,
    disabled: bool,
    // Custom styling overrides
    custom_bg: Option<(u32, u32, u32, u32)>,
    custom_text_color: Option<(u8, u8, u8, u8)>,
    custom_font_size: Option<f32>,
    custom_font_weight: Option<u16>,
    custom_font_family: Option<String>,
    custom_corner_radius: Option<f32>,
}

impl Button {
    /// Create a new button with the given label
    pub fn new(label: impl Into<String>) -> Self {
        let label = label.into();
        let view = Self::build_view(
            &label,
            ButtonVariant::Primary,
            ButtonSize::Medium,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
        );

        Self {
            view,
            label,
            variant: ButtonVariant::Primary,
            size: ButtonSize::Medium,
            disabled: false,
            custom_bg: None,
            custom_text_color: None,
            custom_font_size: None,
            custom_font_weight: None,
            custom_font_family: None,
            custom_corner_radius: None,
        }
    }

    fn build_view(
        label: &str,
        variant: ButtonVariant,
        size: ButtonSize,
        disabled: bool,
        custom_bg: Option<(u32, u32, u32, u32)>,
        custom_text_color: Option<(u8, u8, u8, u8)>,
        custom_font_size: Option<f32>,
        custom_font_weight: Option<u16>,
        custom_font_family: Option<&str>,
        custom_corner_radius: Option<f32>,
    ) -> View {
        let (padding_h, padding_v, font_size) = match size {
            ButtonSize::Small => (12.0, 6.0, 12.0),
            ButtonSize::Medium => (16.0, 10.0, 14.0),
            ButtonSize::Large => (24.0, 12.0, 16.0),
        };

        // Get style for current state
        let style = get_variant_style(variant);
        let state = if disabled {
            ButtonState::Disabled
        } else {
            ButtonState::Normal
        };
        let (variant_bg, variant_text_color) = style.for_state(state);

        // Apply custom overrides
        let bg_color = custom_bg.unwrap_or(variant_bg);
        let text_color = custom_text_color.unwrap_or(variant_text_color);
        let font_size = custom_font_size.unwrap_or(font_size);
        let corner_radius = custom_corner_radius.unwrap_or(match size {
            ButtonSize::Small => 6.0,
            ButtonSize::Medium => 8.0,
            ButtonSize::Large => 10.0,
        });

        // Build the inner content (Row with text)
        let inner_row = Row::new()
            .main_axis_alignment(MainAxisAlignment::Center)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .padding(Padding::symmetric(padding_h, padding_v))
            .width(SizeUnit::Percent(100.0)); // Fill button width for centering

        let font_weight = custom_font_weight.unwrap_or(400);
        let mut text_builder = Text::new()
            .value(label.to_string())
            .font_size(font_size)
            .font_weight(font_weight)
            .text_color(text_color);
        if let Some(family) = custom_font_family {
            text_builder = text_builder.font_family(family);
        }
        let text = text_builder;

        let inner_row = inner_row.child(text);

        // Build the button view
        let button_view = View::new().color(bg_color).border_radius(corner_radius);

        // Add border for Outline variant
        let button_view = if variant == ButtonVariant::Outline {
            let border_color = if disabled {
                (113u8, 119, 134, 255) // Gray border when disabled
            } else {
                (0u8, 88, 188, 255) // Primary blue border
            };
            button_view.border_width(1.0).border_color(border_color)
        } else {
            button_view
        };

        button_view.child(inner_row.node_id())
    }

    fn rebuild_view(&mut self) {
        self.view = Self::build_view(
            &self.label,
            self.variant,
            self.size,
            self.disabled,
            self.custom_bg,
            self.custom_text_color,
            self.custom_font_size,
            self.custom_font_weight,
            self.custom_font_family.as_deref(),
            self.custom_corner_radius,
        );
    }

    /// Set the button variant
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self.rebuild_view();
        self
    }

    /// Set the button size
    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self.rebuild_view();
        self
    }

    /// Set the button label
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self.rebuild_view();
        self
    }

    /// Set disabled state
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self.rebuild_view();
        self
    }

    /// Set a tap handler with color-based press feedback
    pub fn on_tap<F>(self, mut handler: F) -> Self
    where
        F: FnMut(GestureEvent) + 'static,
    {
        let id = self.node_id();
        let variant = self.variant;
        let size = self.size;
        let disabled = self.disabled;

        // Store disabled state for pointer handler checks
        BUTTON_DISABLED.with(|d| {
            d.borrow_mut().insert(id, disabled);
        });

        // If disabled, don't register any handlers
        if disabled {
            return self;
        }

        select_node(id);
        // Register for tap events
        push_command!(SHARED_BUFFER, AttachClick, id);
        push_command!(SHARED_BUFFER, RegisterTapHandler, id, 1u32);

        // Register for pointer events (for press feedback)
        push_command!(SHARED_BUFFER, RegisterPointerDownHandler, id);
        push_command!(SHARED_BUFFER, RegisterPointerUpHandler, id);

        // Store tap handler
        TAP_HANDLERS.with(|h| {
            let mut handlers = h.borrow_mut();
            let entry = handlers.entry(id).or_insert_with(TapHandlerEntry::new);
            entry.single_tap = Some(Box::new(move |e| handler(e)));
        });

        // Store pointer down handler for press effect
        POINTER_DOWN_HANDLERS.with(|h| {
            let mut handlers = h.borrow_mut();
            handlers.insert(
                id,
                Box::new(move |_e| {
                    apply_button_pressed_state(id, variant, size);
                }),
            );
        });

        // Store pointer up handler for release effect
        POINTER_UP_HANDLERS.with(|h| {
            let mut handlers = h.borrow_mut();
            handlers.insert(
                id,
                Box::new(move |_e| {
                    apply_button_normal_state(id, variant, size);
                }),
            );
        });

        self
    }

    /// Set a simple tap handler (no event argument) with press feedback
    pub fn on_click<F>(self, handler: F) -> Self
    where
        F: Fn() + 'static,
    {
        self.on_tap(move |_event| handler())
    }

    /// Set fixed width
    pub fn width(mut self, width: impl Into<SizeUnit>) -> Self {
        let w: SizeUnit = width.into();
        self.view = BaseView::width(self.view, w);
        self
    }

    /// Set fixed height
    pub fn height(mut self, height: impl Into<SizeUnit>) -> Self {
        let h: SizeUnit = height.into();
        self.view = BaseView::height(self.view, h);
        self
    }

    /// Make the button expand to fill available width
    pub fn expanded(mut self) -> Self {
        self.view = BaseView::width(self.view, SizeUnit::Percent(100.0));
        self
    }

    /// Set custom background color (overrides variant)
    pub fn background(mut self, color: impl Into<(u32, u32, u32, u32)>) -> Self {
        self.custom_bg = Some(color.into());
        self.rebuild_view();
        self
    }

    /// Set custom text color (overrides variant)
    pub fn text_color(mut self, color: impl Into<(u8, u8, u8, u8)>) -> Self {
        self.custom_text_color = Some(color.into());
        self.rebuild_view();
        self
    }

    /// Set custom font size (overrides size)
    pub fn font_size(mut self, size: f32) -> Self {
        self.custom_font_size = Some(size);
        self.rebuild_view();
        self
    }

    /// Set custom font weight (overrides default 400)
    pub fn font_weight(mut self, weight: u16) -> Self {
        self.custom_font_weight = Some(weight);
        self.rebuild_view();
        self
    }

    /// Set custom font family
    pub fn font_family(mut self, family: impl Into<String>) -> Self {
        self.custom_font_family = Some(family.into());
        self.rebuild_view();
        self
    }

    /// Set custom corner radius (overrides size default)
    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.custom_corner_radius = Some(radius);
        self.rebuild_view();
        self
    }
}

impl BaseView for Button {
    fn node_id(&self) -> u32 {
        self.view.node_id()
    }
}

// ===== Convenience constructors =====

/// Create a primary button
pub fn primary_button(label: impl Into<String>) -> Button {
    Button::new(label).variant(ButtonVariant::Primary)
}

/// Create a secondary button
pub fn secondary_button(label: impl Into<String>) -> Button {
    Button::new(label).variant(ButtonVariant::Secondary)
}

/// Create an outline button
pub fn outline_button(label: impl Into<String>) -> Button {
    Button::new(label).variant(ButtonVariant::Outline)
}

/// Create a ghost button
pub fn ghost_button(label: impl Into<String>) -> Button {
    Button::new(label).variant(ButtonVariant::Ghost)
}
