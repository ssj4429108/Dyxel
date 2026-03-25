// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Text editor integration based on Parley's PlainEditor

use parley::{FontContext, LayoutContext, PlainEditor, StyleProperty, GenericFamily, FontStack};

pub mod input;
use parley::layout::PositionedLayoutItem;
use peniko::{Brush, Color, Fill};
use vello::kurbo::Affine;
use vello::Scene;

pub use parley::editing::Generation;

/// Text editor with cursor, selection, and layout
pub struct Editor {
    font_cx: FontContext,
    layout_cx: LayoutContext<Brush>,
    editor: PlainEditor<Brush>,
    cursor_visible: bool,
}

impl Editor {
    /// Create new editor with default font size
    pub fn new(font_size: f32) -> Self {
        let mut editor = PlainEditor::new(font_size);
        editor.set_scale(1.0);
        
        // Set default styles
        let styles = editor.edit_styles();
        styles.insert(StyleProperty::Brush(Brush::Solid(Color::WHITE)));
        // Set default font family to system UI font for consistent metrics
        styles.insert(StyleProperty::FontStack(FontStack::Single(GenericFamily::SystemUi.into())));
        
        Self {
            font_cx: FontContext::default(),
            layout_cx: LayoutContext::default(),
            editor,
            cursor_visible: true,
        }
    }
    
    /// Create editor with initial text
    pub fn with_text(mut self, text: &str) -> Self {
        self.editor.set_text(text);
        self
    }
    
    /// Set text content
    pub fn set_text(&mut self, text: &str) {
        self.editor.set_text(text);
    }
    
    /// Get current text
    pub fn text(&self) -> String {
        self.editor.text().to_string()
    }
    
    /// Set font size (rebuilds editor)
    pub fn set_font_size(&mut self, size: f32) {
        // PlainEditor doesn't support changing font size after creation
        // We need to update the style
        self.editor.edit_styles().insert(StyleProperty::FontSize(size));
    }
    
    /// Set text color
    pub fn set_text_color(&mut self, color: Color) {
        self.editor.edit_styles().insert(StyleProperty::Brush(Brush::Solid(color)));
    }
    
    /// Set layout width (for line wrapping)
    pub fn set_width(&mut self, width: Option<f32>) {
        self.editor.set_width(width);
    }
    
    /// Get current generation (for dirty checking)
    pub fn generation(&self) -> Generation {
        self.editor.generation()
    }
    
    /// Show/hide cursor
    pub fn set_cursor_visible(&mut self, visible: bool) {
        self.cursor_visible = visible;
    }
    
    /// Toggle cursor visibility (for blinking)
    pub fn toggle_cursor(&mut self) {
        self.cursor_visible = !self.cursor_visible;
    }
    
    // === Cursor Operations ===
    
    /// Move cursor to point
    pub fn move_to_point(&mut self, x: f32, y: f32) {
        self.editor.driver(&mut self.font_cx, &mut self.layout_cx)
            .move_to_point(x, y);
    }
    
    /// Move cursor left
    pub fn move_left(&mut self) {
        self.editor.driver(&mut self.font_cx, &mut self.layout_cx).move_left();
    }
    
    /// Move cursor right
    pub fn move_right(&mut self) {
        self.editor.driver(&mut self.font_cx, &mut self.layout_cx).move_right();
    }
    
    /// Move cursor up
    pub fn move_up(&mut self) {
        self.editor.driver(&mut self.font_cx, &mut self.layout_cx).move_up();
    }
    
    /// Move cursor down
    pub fn move_down(&mut self) {
        self.editor.driver(&mut self.font_cx, &mut self.layout_cx).move_down();
    }
    
    /// Move to line start
    pub fn move_to_line_start(&mut self) {
        self.editor.driver(&mut self.font_cx, &mut self.layout_cx).move_to_line_start();
    }
    
    /// Move to line end
    pub fn move_to_line_end(&mut self) {
        self.editor.driver(&mut self.font_cx, &mut self.layout_cx).move_to_line_end();
    }
    
    /// Move to text start
    pub fn move_to_text_start(&mut self) {
        self.editor.driver(&mut self.font_cx, &mut self.layout_cx).move_to_text_start();
    }
    
    /// Move to text end
    pub fn move_to_text_end(&mut self) {
        self.editor.driver(&mut self.font_cx, &mut self.layout_cx).move_to_text_end();
    }
    
    // === Selection Operations ===
    
    /// Select all
    pub fn select_all(&mut self) {
        self.editor.driver(&mut self.font_cx, &mut self.layout_cx).select_all();
    }
    
    /// Collapse selection (clear selection)
    pub fn collapse_selection(&mut self) {
        self.editor.driver(&mut self.font_cx, &mut self.layout_cx).collapse_selection();
    }
    
    /// Select word at point
    pub fn select_word_at_point(&mut self, x: f32, y: f32) {
        self.editor.driver(&mut self.font_cx, &mut self.layout_cx)
            .select_word_at_point(x, y);
    }
    
    /// Select left
    pub fn select_left(&mut self) {
        self.editor.driver(&mut self.font_cx, &mut self.layout_cx).select_left();
    }
    
    /// Select right
    pub fn select_right(&mut self) {
        self.editor.driver(&mut self.font_cx, &mut self.layout_cx).select_right();
    }
    
    /// Select up
    pub fn select_up(&mut self) {
        self.editor.driver(&mut self.font_cx, &mut self.layout_cx).select_up();
    }
    
    /// Select down
    pub fn select_down(&mut self) {
        self.editor.driver(&mut self.font_cx, &mut self.layout_cx).select_down();
    }
    
    /// Extend selection to point (drag selection)
    pub fn extend_selection_to_point(&mut self, x: f32, y: f32) {
        self.editor.driver(&mut self.font_cx, &mut self.layout_cx)
            .extend_selection_to_point(x, y);
    }
    
    // === Text Editing ===
    
    /// Insert text at cursor
    pub fn insert(&mut self, text: &str) {
        self.editor.driver(&mut self.font_cx, &mut self.layout_cx)
            .insert_or_replace_selection(text);
    }
    
    /// Delete forward
    pub fn delete(&mut self) {
        self.editor.driver(&mut self.font_cx, &mut self.layout_cx).delete();
    }
    
    /// Delete backward
    pub fn backspace(&mut self) {
        self.editor.driver(&mut self.font_cx, &mut self.layout_cx).backdelete();
    }
    
    // === Rendering ===
    
    /// Draw the editor content into the scene
    /// 
    /// `transform` is the base transform (translation to editor position)
    /// Returns the current generation for dirty checking
    pub fn draw(&mut self, scene: &mut Scene, transform: Affine) -> Generation {
        
        // 1. Draw selection background
        self.editor.selection_geometry_with(|rect, _| {
            let rect = vello::kurbo::Rect::new(
                rect.x0, rect.y0, rect.x1, rect.y1
            );
            scene.fill(
                Fill::NonZero,
                transform,
                Color::from_rgb8(100, 150, 255), // Selection color
                None,
                &rect,
            );
        });
        
        // 2. Draw cursor
        if self.cursor_visible {
            if let Some(cursor) = self.editor.cursor_geometry(1.5) {
                let cursor_rect = vello::kurbo::Rect::new(
                    cursor.x0, cursor.y0, cursor.x1, cursor.y1
                );
                scene.fill(
                    Fill::NonZero,
                    transform,
                    Color::WHITE,
                    None,
                    &cursor_rect,
                );
            }
        }
        
        // 3. Draw text using draw_glyphs
        let layout = self.editor.layout(&mut self.font_cx, &mut self.layout_cx);
        
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(glyph_run) = item else { continue };
                
                let style = glyph_run.style();
                let run = glyph_run.run();
                let font = run.font();
                let font_size = run.font_size();
                
                // Build glyph iterator
                let mut x = glyph_run.offset();
                let y = glyph_run.baseline();
                let synthesis = run.synthesis();
                let glyph_xform = synthesis
                    .skew()
                    .map(|angle| Affine::skew(angle.to_radians().tan() as f64, 0.0));
                
                // Get the line height for Android offset calculation
                #[cfg(target_os = "android")]
                let line_height = layout.height();
                
                let glyphs = glyph_run.glyphs().map(|glyph| {
                    let gx = x + glyph.x;
                    let gy = y + glyph.y;
                    x += glyph.advance;
                    vello::Glyph {
                        id: glyph.id,
                        x: gx,
                        y: gy,
                    }
                });
                
                // Use vello's draw_glyphs (chain all methods since they consume self)
                scene
                    .draw_glyphs(font)
                    .brush(&style.brush)
                    .hint(true)
                    .transform(transform)
                    .font_size(font_size)
                    .glyph_transform(glyph_xform)
                    .normalized_coords(run.normalized_coords())
                    .draw(Fill::NonZero, glyphs);
            }
        }
        
        self.editor.generation()
    }
    
    /// Get layout dimensions (width, height)
    pub fn layout_size(&mut self) -> (f32, f32) {
        let layout = self.editor.layout(&mut self.font_cx, &mut self.layout_cx);
        (layout.width(), layout.height())
    }
}

/// Helper struct for managing multiple editors or editor state
pub struct EditorState {
    pub editor: Editor,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    last_generation: Generation,
}

impl EditorState {
    pub fn new(editor: Editor, x: f32, y: f32, width: f32, height: f32) -> Self {
        let last_generation = editor.generation();
        Self {
            editor,
            x,
            y,
            width,
            height,
            last_generation,
        }
    }
    
    /// Check if editor needs redraw
    pub fn needs_redraw(&self) -> bool {
        self.editor.generation() != self.last_generation
    }
    
    /// Mark as drawn (update generation)
    pub fn mark_drawn(&mut self) {
        self.last_generation = self.editor.generation();
    }
    
    /// Draw if needed, returns true if drew
    pub fn draw_if_needed(&mut self, scene: &mut Scene) -> bool {
        if self.needs_redraw() || self.editor.cursor_visible {
            let transform = Affine::translate((self.x as f64, self.y as f64));
            self.editor.draw(scene, transform);
            self.mark_drawn();
            true
        } else {
            false
        }
    }
    
    /// Convert global point to local editor coordinates
    pub fn to_local(&self, global_x: f32, global_y: f32) -> (f32, f32) {
        (global_x - self.x, global_y - self.y)
    }
    
    /// Check if point is inside editor
    pub fn hit_test(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}
