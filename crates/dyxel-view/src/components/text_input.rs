// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! TextInput Component - Reactive version with Focus support
//!
//! Implementation based on Task 4 plan (2026-04-09).

use crate::focus::{FocusCapability, FocusableView};
use crate::Prop;
use crate::{push_command, select_node, track_node, BaseView, TextRenderable, SHARED_BUFFER};
use dyxel_shared::TextState;
use futures_signals::signal::SignalExt;
use std::sync::atomic::Ordering;

/// TextInput component - A Text node with input and focus capabilities
pub struct TextInput {
    pub id: u32,
    placeholder_text: Option<String>,
}

impl TextInput {
    /// Create a new TextInput node
    pub fn new() -> Self {
        let id = crate::NODE_COUNTER.fetch_add(1, Ordering::SeqCst);
        track_node(id);

        // Create as Text node (for rendering)
        push_command!(SHARED_BUFFER, CreateTextNode, id);
        select_node(id);

        // Register as text input (enables keyboard, cursor, selection)
        push_command!(SHARED_BUFFER, CreateTextInput, id);

        // Set default styles
        select_node(id);
        push_command!(SHARED_BUFFER, SetTextColor, id, 0u8, 0u8, 0u8, 255u8);
        push_command!(SHARED_BUFFER, SetFontSize, id, 16.0_f32);

        let this = Self {
            id,
            placeholder_text: None,
        };

        // Register with focus manager - must be done explicitly before on_tap
        let caps = this.focus_capabilities();
        crate::focus::register_focusable(id, caps);

        // Register tap handler for focus (empty user handler) and return
        this.on_tap(|_| {})
    }

    /// Set the text state value (Responsive)
    pub fn value(self, state: impl Into<Prop<TextState>>) -> Self {
        match state.into() {
            Prop::Static(v) => {
                let text = v.text.clone();
                let sel = v.selection.clone();
                select_node(self.id);
                push_command!(
                    SHARED_BUFFER,
                    SyncTextState,
                    self.id,
                    text.len() as u32,
                    sel.start as u32,
                    sel.end as u32
                );
                unsafe {
                    let offset = SHARED_BUFFER.command_len as usize;
                    if offset + text.len() <= dyxel_shared::MAX_COMMAND_BYTES {
                        SHARED_BUFFER.command_data[offset..offset + text.len()]
                            .copy_from_slice(text.as_bytes());
                        SHARED_BUFFER.command_len = (offset + text.len()) as u32;
                    }
                }
                // Keep local cache in sync so keyboard input appends correctly
                crate::update_text_input_cache(self.id, text);
            }
            Prop::Dynamic(s) => {
                let id = self.id;
                let future = s.for_each(move |val| {
                    let text = val.text.clone();
                    let sel = val.selection.clone();
                    select_node(id);
                    push_command!(
                        SHARED_BUFFER,
                        SyncTextState,
                        id,
                        text.len() as u32,
                        sel.start as u32,
                        sel.end as u32
                    );
                    unsafe {
                        let offset = SHARED_BUFFER.command_len as usize;
                        if offset + text.len() <= dyxel_shared::MAX_COMMAND_BYTES {
                            SHARED_BUFFER.command_data[offset..offset + text.len()]
                                .copy_from_slice(text.as_bytes());
                            SHARED_BUFFER.command_len = (offset + text.len()) as u32;
                        }
                    }
                    // Keep local cache in sync so keyboard input appends correctly
                    crate::update_text_input_cache(id, text);
                    async {}
                });
                crate::spawn(Box::pin(future));
            }
        }
        self
    }

    /// Callback when text state changes (Responsive)
    pub fn on_change<F>(self, mut handler: F) -> Self
    where
        F: FnMut(TextState) + 'static,
    {
        crate::TEXT_INPUT_HANDLERS.with(|h| {
            h.borrow_mut().insert(
                self.id,
                Box::new(move |new_state| {
                    handler(new_state);
                }),
            );
        });
        self
    }

    /// Set font size
    pub fn font_size(self, size: impl Into<Prop<f32>>) -> Self {
        crate::apply_prop(self.id, size.into(), |id, s| {
            select_node(id);
            push_command!(SHARED_BUFFER, SetFontSize, id, s);
        });
        self
    }

    /// Set text color
    pub fn text_color(self, color: impl Into<Prop<(u8, u8, u8, u8)>>) -> Self {
        crate::apply_prop(self.id, color.into(), |id, (r, g, b, a)| {
            select_node(id);
            push_command!(SHARED_BUFFER, SetTextColor, id, r, g, b, a);
        });
        self
    }

    /// Set the placeholder text
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        let text_str = text.into();
        self.placeholder_text = Some(text_str.clone());

        let id = self.id;
        select_node(id);
        let len = text_str.len() as u32;
        push_command!(SHARED_BUFFER, SetTextInputPlaceholder, id, len);
        unsafe {
            let offset = SHARED_BUFFER.command_len as usize;
            if offset + text_str.len() <= dyxel_shared::MAX_COMMAND_BYTES {
                SHARED_BUFFER.command_data[offset..offset + text_str.len()]
                    .copy_from_slice(text_str.as_bytes());
                SHARED_BUFFER.command_len = (offset + text_str.len()) as u32;
            }
        }
        self
    }
}

impl Default for TextInput {
    fn default() -> Self {
        Self::new()
    }
}

impl BaseView for TextInput {
    fn node_id(&self) -> u32 {
        self.id
    }

    /// Override on_tap to handle focus
    fn on_tap(self, mut handler: impl FnMut(crate::gesture::GestureEvent) + 'static) -> Self
    where
        Self: Sized,
    {
        let id = self.id;

        // Internal focus handler
        let internal_handler = move |e: crate::gesture::GestureEvent| {
            // Handle focus logic - request focus if not already focused
            // Note: request_focus returns false if already focused or node not registered
            let need_focus = !crate::focus::is_focused(id);
            if need_focus {
                if crate::focus::request_focus(id, FocusCapability::Keyboard) {
                    select_node(id);
                    push_command!(SHARED_BUFFER, SetTextInputFocused, id, 1u8);
                    push_command!(SHARED_BUFFER, ShowTextInputKeyboard);
                }
            }
            // Execute user handler
            handler(e);
        };

        // Register tap handler
        select_node(id);
        push_command!(SHARED_BUFFER, AttachClick, id);
        push_command!(SHARED_BUFFER, RegisterTapHandler, id, 1u32);
        crate::TAP_HANDLERS.with(|h| {
            let mut handlers = h.borrow_mut();
            let entry = handlers
                .entry(id)
                .or_insert_with(crate::TapHandlerEntry::new);
            entry.single_tap = Some(Box::new(internal_handler));
        });
        self
    }
}

impl TextRenderable for TextInput {
    fn text_node_id(&self) -> u32 {
        self.id
    }
}

impl FocusableView for TextInput {
    fn focus_capabilities(&self) -> Vec<FocusCapability> {
        vec![FocusCapability::Keyboard]
    }

    fn on_focus(&self, _capability: FocusCapability) {
        // Focus is handled in on_tap to coordinate with command buffer
    }

    fn on_blur(&self, _capability: FocusCapability) {
        // Blur is handled by the Host via bridge commands
    }
}
