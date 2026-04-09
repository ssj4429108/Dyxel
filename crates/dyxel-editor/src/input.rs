// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Input event handling for Editor

use crate::Editor;

/// Keyboard key
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Character(char),
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Tab,
    Escape,
}

/// Modifier keys state
#[derive(Debug, Clone, Copy, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

/// Keyboard event
#[derive(Debug, Clone)]
pub struct KeyboardEvent {
    pub key: Key,
    pub modifiers: Modifiers,
    pub pressed: bool, // true = key down, false = key up
}

/// Pointer (mouse/touch) event
#[derive(Debug, Clone)]
pub struct PointerEvent {
    pub x: f32,
    pub y: f32,
    pub pressed: bool,
    pub button: PointerButton,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerButton {
    None,
    Primary,   // Left mouse, touch
    Secondary, // Right mouse
    Middle,
}

impl Editor {
    /// Handle keyboard event
    pub fn handle_keyboard(&mut self, event: &KeyboardEvent) {
        if !event.pressed {
            return; // Only handle key down for now
        }

        let shift = event.modifiers.shift;
        let ctrl = event.modifiers.ctrl;
        let meta = event.modifiers.meta;
        let action_mod = if cfg!(target_os = "macos") {
            meta
        } else {
            ctrl
        };

        match event.key {
            Key::Character(c) => {
                if action_mod {
                    // Ctrl/Cmd shortcuts
                    match c.to_ascii_lowercase() {
                        'a' if !shift => self.select_all(),
                        'a' if shift => self.collapse_selection(),
                        _ => {} // Other shortcuts not handled
                    }
                } else {
                    self.insert(&c.to_string());
                }
            }
            Key::Enter => self.insert("\n"),
            Key::Backspace => {
                if action_mod {
                    // Could implement word backdelete
                    self.backspace();
                } else {
                    self.backspace();
                }
            }
            Key::Delete => {
                if action_mod {
                    // Could implement word delete
                    self.delete();
                } else {
                    self.delete();
                }
            }
            Key::Left => {
                if action_mod {
                    if shift {
                        self.select_word_left();
                    } else {
                        self.move_word_left();
                    }
                } else if shift {
                    self.select_left();
                } else {
                    self.move_left();
                }
            }
            Key::Right => {
                if action_mod {
                    if shift {
                        self.select_word_right();
                    } else {
                        self.move_word_right();
                    }
                } else if shift {
                    self.select_right();
                } else {
                    self.move_right();
                }
            }
            Key::Up => {
                if shift {
                    self.select_up();
                } else {
                    self.move_up();
                }
            }
            Key::Down => {
                if shift {
                    self.select_down();
                } else {
                    self.move_down();
                }
            }
            Key::Home => {
                if action_mod {
                    if shift {
                        self.select_to_text_start();
                    } else {
                        self.move_to_text_start();
                    }
                } else if shift {
                    self.select_to_line_start();
                } else {
                    self.move_to_line_start();
                }
            }
            Key::End => {
                if action_mod {
                    if shift {
                        self.select_to_text_end();
                    } else {
                        self.move_to_text_end();
                    }
                } else if shift {
                    self.select_to_line_end();
                } else {
                    self.move_to_line_end();
                }
            }
            _ => {}
        }
    }

    /// Handle pointer (mouse/touch) event
    pub fn handle_pointer(&mut self, event: &PointerEvent) {
        if event.button != PointerButton::Primary && event.button != PointerButton::None {
            return;
        }

        if event.pressed {
            self.move_to_point(event.x, event.y);
        }
    }

    /// Handle double-click (select word)
    pub fn handle_double_click(&mut self, x: f32, y: f32) {
        self.select_word_at_point(x, y);
    }

    /// Handle drag (extend selection)
    pub fn handle_drag(&mut self, x: f32, y: f32) {
        self.extend_selection_to_point(x, y);
    }
}

// Word selection helpers (implemented using existing methods)
impl Editor {
    fn select_word_left(&mut self) {
        // Move word left, keeping selection
        // Parley doesn't have direct API, use point-based selection
        // This is a simplified implementation
        self.select_left(); // Fallback
    }

    fn select_word_right(&mut self) {
        self.select_right(); // Fallback
    }

    fn move_word_left(&mut self) {
        self.move_left(); // Fallback
    }

    fn move_word_right(&mut self) {
        self.move_right(); // Fallback
    }

    fn select_to_line_start(&mut self) {
        self.move_to_line_start();
    }

    fn select_to_line_end(&mut self) {
        self.move_to_line_end();
    }

    fn select_to_text_start(&mut self) {
        self.move_to_text_start();
    }

    fn select_to_text_end(&mut self) {
        self.move_to_text_end();
    }
}
