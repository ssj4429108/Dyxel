// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! TextRenderable Trait - Shared text rendering interface for Text and TextInput

use crate::{push_command, select_node, Prop, SHARED_BUFFER};

/// Trait for components that render text using dyxel-editor
///
/// This trait is implemented by both `Text` and `TextInput`, allowing them to share
/// the same text styling and rendering infrastructure.
pub trait TextRenderable: Sized {
    /// Get the node ID for this text component
    fn text_node_id(&self) -> u32;

    /// Set the text content
    fn text_value(self, p: impl Into<Prop<String>>) -> Self {
        let id = self.text_node_id();
        crate::apply_prop(id, p.into(), |node_id, s: String| {
            select_node(node_id);
            let len = s.len() as u32;
            unsafe {
                push_command!(SHARED_BUFFER, SetTextContent, node_id, len);
                let offset = SHARED_BUFFER.command_len as usize;
                if offset + s.len() <= dyxel_shared::MAX_COMMAND_BYTES {
                    SHARED_BUFFER.command_data[offset..offset + s.len()]
                        .copy_from_slice(s.as_bytes());
                    SHARED_BUFFER.command_len = (offset + s.len()) as u32;
                }
            }
        });
        self
    }

    /// Set the font size in logical pixels
    fn font_size(self, p: impl Into<Prop<f32>>) -> Self {
        let id = self.text_node_id();
        crate::apply_prop(id, p.into(), |node_id, size| {
            select_node(node_id);
            push_command!(SHARED_BUFFER, SetFontSize, node_id, size);
        });
        self
    }

    /// Set the font weight (400 = normal, 700 = bold)
    fn font_weight(self, p: impl Into<Prop<u16>>) -> Self {
        let id = self.text_node_id();
        crate::apply_prop(id, p.into(), |node_id, weight| {
            select_node(node_id);
            push_command!(SHARED_BUFFER, SetTextWeight, node_id, weight);
        });
        self
    }

    /// Set the text color as (r, g, b, a) tuple
    fn text_color(self, p: impl Into<Prop<(u8, u8, u8, u8)>>) -> Self {
        let id = self.text_node_id();
        crate::apply_prop(id, p.into(), |node_id, (r, g, b, a)| {
            select_node(node_id);
            push_command!(SHARED_BUFFER, SetTextColor, node_id, r, g, b, a);
        });
        self
    }

    /// Set the font family (e.g., "Inter", "System")
    fn font_family(self, p: impl Into<Prop<String>>) -> Self {
        let id = self.text_node_id();
        crate::apply_prop(id, p.into(), |node_id, family: String| {
            select_node(node_id);
            let len = family.len() as u32;
            push_command!(SHARED_BUFFER, SetTextFontFamily, node_id, len);
            unsafe {
                let offset = SHARED_BUFFER.command_len as usize;
                if offset + family.len() <= dyxel_shared::MAX_COMMAND_BYTES {
                    SHARED_BUFFER.command_data[offset..offset + family.len()]
                        .copy_from_slice(family.as_bytes());
                    SHARED_BUFFER.command_len = (offset + family.len()) as u32;
                }
            }
        });
        self
    }

    /// Set the text alignment (0 = start, 1 = center, 2 = end, 3 = justified)
    fn text_align(self, p: impl Into<Prop<u8>>) -> Self {
        let id = self.text_node_id();
        crate::apply_prop(id, p.into(), |node_id, align| {
            select_node(node_id);
            push_command!(SHARED_BUFFER, SetTextAlign, node_id, align);
        });
        self
    }

    /// Set line height multiplier (1.0 = normal, 1.5 = relaxed)
    fn line_height(self, _p: impl Into<Prop<f32>>) -> Self {
        // TODO: Implement line height in protocol
        self
    }

    /// Set letter spacing in pixels
    fn letter_spacing(self, _p: impl Into<Prop<f32>>) -> Self {
        // TODO: Implement letter spacing in protocol
        self
    }

    /// Set maximum number of lines (0 = unlimited)
    fn max_lines(self, _p: impl Into<Prop<u32>>) -> Self {
        // TODO: Implement max lines (requires text truncation)
        self
    }

    /// Set text overflow behavior
    fn text_overflow(self, _p: impl Into<Prop<TextOverflow>>) -> Self {
        // TODO: Implement text overflow (clip, ellipsis)
        self
    }
}

/// Text overflow behavior when content exceeds available space
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextOverflow {
    /// Clip the text at the boundary
    Clip,
    /// Show ellipsis (...) at the end
    Ellipsis,
    /// Fade out at the end
    Fade,
}

impl Default for TextOverflow {
    fn default() -> Self {
        TextOverflow::Clip
    }
}

/// Extension trait for TextRenderable with common convenience methods
pub trait TextRenderableExt: TextRenderable {
    /// Set text color using RGB values (alpha = 255)
    fn color_rgb(self, r: u8, g: u8, b: u8) -> Self {
        self.text_color((r, g, b, 255))
    }

    /// Set text color using hex string (e.g., "#FF0000" or "#FF0000FF")
    fn color_hex(self, hex: &str) -> Self {
        let color = parse_hex_color(hex).unwrap_or((0, 0, 0, 255));
        self.text_color(color)
    }

    /// Make text bold (font_weight = 700)
    fn bold(self) -> Self {
        self.font_weight(700)
    }

    /// Make text italic (requires font family support)
    fn italic(self) -> Self {
        // TODO: Implement italic (may need font style property)
        self
    }

    /// Center align the text
    fn center(self) -> Self
    where
        Self: Sized,
    {
        self.text_align(dyxel_shared::TextAlign::Center)
    }

    /// Right align the text
    fn right(self) -> Self
    where
        Self: Sized,
    {
        self.text_align(dyxel_shared::TextAlign::End)
    }
}

impl<T: TextRenderable> TextRenderableExt for T {}

/// Parse a hex color string into (r, g, b, a) tuple
fn parse_hex_color(hex: &str) -> Option<(u8, u8, u8, u8)> {
    let hex = hex.trim_start_matches('#');
    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some((r, g, b, 255))
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            Some((r, g, b, a))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_color() {
        assert_eq!(parse_hex_color("#FF0000"), Some((255, 0, 0, 255)));
        assert_eq!(parse_hex_color("#00FF00"), Some((0, 255, 0, 255)));
        assert_eq!(parse_hex_color("#0000FF"), Some((0, 0, 255, 255)));
        assert_eq!(parse_hex_color("#FF000080"), Some((255, 0, 0, 128)));
        assert_eq!(parse_hex_color("invalid"), None);
    }
}
