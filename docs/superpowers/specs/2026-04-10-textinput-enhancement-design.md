# TextInput Enhancement Design

**Date:** 2026-04-10  
**Status:** Ready for Implementation  
**Author:** Claude Code

---

## 1. Overview

This design document specifies enhancements to the Dyxel TextInput component and Text component selection capabilities. The goal is to bring TextInput closer to Flutter's TextField functionality while maintaining Dyxel's "Thin Guest, Thick Host" architecture.

### 1.1 Scope

- **TextInput styling:** padding, cursor customization, colors, borders
- **Text selection:** Enable long-press selection for Text components
- **RSX macro fix:** Ensure explicit styles override defaults

### 1.2 Non-Goals

- Rich text editing (spans, mixed styles within single input)
- Complex input validation
- Autocomplete/suggestion UI
- Multi-line text input expansion (TextArea is future work)

---

## 2. Architecture Principles

1. **Reuse existing infrastructure:** Use `SetPadding` (OpCode 13) for container padding
2. **Extend, don't replace:** Add new protocols rather than breaking existing ones
3. **Host-side rendering:** All visual effects (shadows, rounded corners) computed in Host (Vello)
4. **Guest-side API:** WASM exposes builder-pattern API that translates to protocol commands

---

## 3. Design Details

### 3.1 Padding Support

**Decision:** Use existing `SetPadding` instruction (OpCode 13) for container padding.

**WASM API:**
```rust
impl TextInput {
    pub fn padding(self, padding: impl Into<Prop<(f32, f32, f32, f32)>>) -> Self;
    pub fn padding_horizontal(self, value: f32) -> Self;
    pub fn padding_vertical(self, value: f32) -> Self;
    pub fn padding_all(self, value: f32) -> Self;
}
```

**Default:** `(12.0, 16.0, 12.0, 16.0)` (top, right, bottom, left) - iOS-style comfortable spacing

---

### 3.2 Cursor Customization

**Structure:**
```rust
pub struct CursorStyle {
    pub width: f32,           // Default: 2.0
    pub color: [u8; 4],       // Default: inherit text color
    pub radius: f32,          // Default: 0.0 (rectangular), typical: 1.0-2.0
    pub blink_interval_ms: u64, // Default: 530 (iOS style)
}

pub struct ShadowStyle {
    pub color: [u8; 4],       // Default: cursor color with 30% opacity
    pub blur_radius: f32,     // Default: 4.0
    pub offset_x: f32,        // Default: 0.0
    pub offset_y: f32,        // Default: 0.0
}
```

**New Protocol Instructions:**

| OpCode | Name | Parameters |
|--------|------|------------|
| 134 | SetTextInputCursorStyle | id, width, radius, r, g, b, a |
| 135 | SetTextInputCursorShadow | id, blur, offset_x, offset_y, r, g, b, a |
| 136 | SetTextInputCursorBlinkInterval | id, interval_ms |

**WASM API:**
```rust
impl TextInput {
    pub fn cursor_width(self, width: f32) -> Self;
    pub fn cursor_color(self, color: impl Into<Prop<(u8, u8, u8, u8)>>) -> Self;
    pub fn cursor_radius(self, radius: f32) -> Self;
    pub fn cursor_blink_interval(self, ms: u32) -> Self;
    pub fn cursor_shadow(self, shadow: ShadowStyle) -> Self;
    
    // Convenience: set all at once
    pub fn cursor_style(self, style: CursorStyle) -> Self;
}
```

**Rendering:**
- Cursor drawn as filled rectangle with optional rounded corners
- Shadow applied using Vello's shadow/blur effects
- Blink state managed by Host (TextInputManager::update_cursor_blink)

---

### 3.3 Text Styling (RSX Fix)

**Problem:** TextInput::new() hardcodes defaults:
```rust
push_command!(SHARED_BUFFER, SetTextColor, id, 0u8, 0u8, 0u8, 255u8);
push_command!(SHARED_BUFFER, SetFontSize, id, 16.0_f32);
```

**Solution:** Deferred default application

```rust
pub struct TextInput {
    id: u32,
    placeholder_text: Option<String>,
    // Track which styles have been explicitly set
    explicit_styles: RefCell<ExplicitStyles>,
}

struct ExplicitStyles {
    font_size: bool,
    text_color: bool,
    // ... etc
}

impl TextInput {
    pub fn new() -> Self {
        // Do NOT apply defaults here
        // Just create the node
    }
    
    pub fn font_size(self, size: impl Into<Prop<f32>>) -> Self {
        self.explicit_styles.borrow_mut().font_size = true;
        // ... apply prop
        self
    }
    
    // Called by runtime before first render
    pub(crate) fn apply_defaults(&self) {
        let explicit = self.explicit_styles.borrow();
        if !explicit.font_size {
            self.apply_font_size(16.0);
        }
        if !explicit.text_color {
            self.apply_text_color((0, 0, 0, 255));
        }
        // ... etc
    }
}
```

---

### 3.4 Placeholder Styling

Currently placeholder inherits text style. Need independent styling.

**New Protocol:**

| OpCode | Name | Parameters |
|--------|------|------------|
| 131 | SetTextInputPlaceholderStyle | id, r, g, b, a, font_size |

**WASM API:**
```rust
impl TextInput {
    pub fn placeholder_color(self, color: impl Into<Prop<(u8, u8, u8, u8)>>) -> Self;
    pub fn placeholder_font_size(self, size: impl Into<Prop<f32>>) -> Self;
    
    // Existing
    pub fn placeholder(self, text: impl Into<String>) -> Self;
}
```

**Default:** Color `#999999` (muted gray), font_size same as text

---

### 3.5 Container Styling

Background color, border, and border radius for the input container.

**New Protocol:**

| OpCode | Name | Parameters |
|--------|------|------------|
| 132 | SetTextInputBackgroundColor | id, r, g, b, a |
| 133 | SetTextInputBorderStyle | id, style, width, r, g, b, a |

**WASM API:**
```rust
impl TextInput {
    pub fn background_color(self, color: impl Into<Prop<(u8, u8, u8, u8)>>) -> Self;
    pub fn border_width(self, width: f32) -> Self;
    pub fn border_color(self, color: impl Into<Prop<(u8, u8, u8, u8)>>) -> Self;
    pub fn border_radius(self, radius: f32) -> Self;
    
    // Convenience
    pub fn border(self, width: f32, color: impl Into<Prop<(u8, u8, u8, u8)>>) -> Self;
}
```

**Defaults:**
- Background: transparent (use parent background)
- Border: 1px solid `#E0E0E0`
- Border radius: 8.0 (iOS-style)

---

### 3.6 Selection Styling

Visual feedback for selected text.

**New Protocol:**

| OpCode | Name | Parameters |
|--------|------|------------|
| 137 | SetTextInputSelectionColor | id, r, g, b, a |

**WASM API:**
```rust
impl TextInput {
    pub fn selection_color(self, color: impl Into<Prop<(u8, u8, u8, u8)>>) -> Self;
}
```

**Default:** Primary color with 30% opacity (e.g., `#007AFF4D` on iOS)

**Rendering:**
- Fill selected text bounds with semi-transparent color
- Draw cursor at selection boundaries

---

### 3.7 Text Component Selection (SelectableText)

Enable Text to be selectable like Flutter's SelectableText.

**New Protocol:**

| OpCode | Name | Parameters |
|--------|------|------------|
| 140 | SetTextSelectable | id, enabled |
| 141 | SetTextSelection | id, start, end |

**WASM API:**
```rust
impl Text {
    pub fn selectable(self, enabled: bool) -> Self;
    pub fn selection(self, range: impl Into<Prop<(usize, usize)>>) -> Self;
    pub fn selection_color(self, color: impl Into<Prop<(u8, u8, u8, u8)>>) -> Self;
}
```

**Behavior:**
- When `selectable=true`, long-press enters selection mode
- Shows selection handles (magnifying glass on iOS)
- Context menu: Copy, Select All
- No cursor blinking (static cursor at selection start)

**Sharing with TextInput:**
Both use same selection rendering code in Vello backend.

---

## 4. Render State Updates

### 4.1 TextInputRenderState (dyxel-render-api)

```rust
pub struct TextInputRenderState {
    // Existing fields
    pub text: String,
    pub focused: bool,
    pub cursor_pos: usize,
    pub selection_start: usize,
    pub cursor_visible: bool,
    pub secure: bool,
    pub composing_text: String,
    pub is_composing: bool,
    pub composition_start: usize,
    pub placeholder: String,
    
    // New fields
    pub cursor_style: CursorStyle,
    pub cursor_shadow: Option<ShadowStyle>,
    pub selection_color: [u8; 4],
    pub placeholder_style: PlaceholderStyle,
    pub container_style: ContainerStyle,
}

pub struct PlaceholderStyle {
    pub color: [u8; 4],
    pub font_size: f32,
}

pub struct ContainerStyle {
    pub background_color: [u8; 4],
    pub border_width: f32,
    pub border_color: [u8; 4],
    pub border_radius: f32,
    pub padding: [f32; 4], // top, right, bottom, left
}
```

---

## 5. Implementation Plan Summary

### Phase 1: Core Fixes (P0)
1. Fix RSX macro - deferred default application
2. Add padding support using existing SetPadding
3. Basic cursor customization (width, color, radius)

### Phase 2: Enhanced Styling (P1)
4. Cursor shadow effects
5. Placeholder independent styling
6. Container background/border/radius
7. Selection color

### Phase 3: Advanced Features (P2)
8. Text component selection mode
9. Selection handles rendering
10. Animation support (focus border transition)

---

## 6. File Changes

| File | Change |
|------|--------|
| `dyxel-shared/src/protocol.rs` | Add new OpCodes 130-141 |
| `dyxel-view/src/components/text_input.rs` | Add new builder methods, fix RSX |
| `dyxel-view/src/lib.rs` (Text) | Add selectable API |
| `dyxel-render-api/src/lib.rs` | Extend TextInputRenderState |
| `dyxel-render-vello/src/lib.rs` | Update render_cursor, add shadow support |
| `dyxel-core/src/text_input/manager.rs` | Handle new protocol commands |

---

## 7. Backwards Compatibility

All new features are additive:
- New OpCodes don't affect existing command processing
- Default styles match current behavior (black text, 16px, 2px cursor)
- Text selection is opt-in via `selectable(true)`

---

## 8. Testing Strategy

1. **Unit tests:** Protocol encoding/decoding
2. **Integration tests:** WASM API generates correct command sequence
3. **Visual tests:** Render output matches expected cursor/selection appearance
4. **Interaction tests:** Long-press on Text triggers selection mode

---

## 9. Open Questions

1. Should cursor shadow be part of CursorStyle or a separate Layer effect?
2. Do we need content padding separate from container padding?
3. Should selection handles be platform-native or custom rendered?

---

## 10. References

- Flutter TextField: https://api.flutter.dev/flutter/material/TextField-class.html
- Flutter InputDecoration: https://api.flutter.dev/flutter/material/InputDecoration-class.html
- Flutter SelectableText: https://api.flutter.dev/flutter/material/SelectableText-class.html
- iOS UITextField styling guidelines
