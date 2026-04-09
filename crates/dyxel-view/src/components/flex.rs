// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Flex Layout Components
//!
//! Provides high-level flex container components with convenient API.

use crate::{
    select_node, AlignItems, BaseView, FlexDirection, FlexWrap, JustifyContent, SizeUnit, View,
    SHARED_BUFFER,
};
use dyxel_shared::push_command;

// Re-export types needed for flex containers
pub use crate::AlignContent;

/// Main axis alignment for flex containers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainAxisAlignment {
    /// Items are packed toward the start
    Start,
    /// Items are packed toward the end
    End,
    /// Items are centered
    Center,
    /// Items are evenly distributed with equal space between them
    SpaceBetween,
    /// Items are evenly distributed with equal space around them
    SpaceAround,
    /// Items are evenly distributed with equal space including edges
    SpaceEvenly,
}

impl From<MainAxisAlignment> for JustifyContent {
    fn from(align: MainAxisAlignment) -> Self {
        match align {
            MainAxisAlignment::Start => JustifyContent::FlexStart,
            MainAxisAlignment::End => JustifyContent::FlexEnd,
            MainAxisAlignment::Center => JustifyContent::Center,
            MainAxisAlignment::SpaceBetween => JustifyContent::SpaceBetween,
            MainAxisAlignment::SpaceAround => JustifyContent::SpaceAround,
            MainAxisAlignment::SpaceEvenly => JustifyContent::SpaceEvenly,
        }
    }
}

/// Cross axis alignment for flex containers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossAxisAlignment {
    /// Items are aligned to the start
    Start,
    /// Items are aligned to the end
    End,
    /// Items are centered
    Center,
    /// Items stretch to fill the container
    Stretch,
}

impl From<CrossAxisAlignment> for AlignItems {
    fn from(align: CrossAxisAlignment) -> Self {
        match align {
            CrossAxisAlignment::Start => AlignItems::FlexStart,
            CrossAxisAlignment::End => AlignItems::FlexEnd,
            CrossAxisAlignment::Center => AlignItems::Center,
            CrossAxisAlignment::Stretch => AlignItems::Stretch,
        }
    }
}

/// Common flex container properties
#[derive(Debug, Clone)]
pub struct FlexContainerProps {
    pub main_axis_alignment: MainAxisAlignment,
    pub cross_axis_alignment: CrossAxisAlignment,
    pub spacing: f32,
    pub padding: (f32, f32, f32, f32),
    pub width: Option<SizeUnit>,
    pub height: Option<SizeUnit>,
    pub background: Option<(u32, u32, u32, u32)>,
    pub corner_radius: f32,
    pub clip_to_bounds: bool,
}

impl Default for FlexContainerProps {
    fn default() -> Self {
        Self {
            main_axis_alignment: MainAxisAlignment::Start,
            cross_axis_alignment: CrossAxisAlignment::Stretch,
            spacing: 0.0,
            padding: (0.0, 0.0, 0.0, 0.0),
            width: None,
            height: None,
            background: None,
            corner_radius: 0.0,
            clip_to_bounds: false,
        }
    }
}

/// Column - vertical flex container
///
/// # Example
/// ```rust,ignore
/// Column::new()
///     .spacing(16.0)
///     .main_axis_alignment(MainAxisAlignment::Center)
///     .child(Text("Item 1"))
///     .child(Text("Item 2"))
/// ```
pub struct Column {
    view: Option<View>,
    props: FlexContainerProps,
    children: Vec<u32>,
}

impl Column {
    pub fn new() -> Self {
        let props = FlexContainerProps::default();
        Self {
            view: None,
            props,
            children: Vec::new(),
        }
    }

    fn build_view(props: &FlexContainerProps, direction: FlexDirection) -> View {
        let view = View::new();
        let id = view.node_id();

        // Apply flex properties directly via commands
        select_node(id);
        push_command!(SHARED_BUFFER, SetFlexDirection, id, direction as u32);
        push_command!(
            SHARED_BUFFER,
            SetJustifyContent,
            id,
            Into::<JustifyContent>::into(props.main_axis_alignment) as u32
        );
        push_command!(
            SHARED_BUFFER,
            SetAlignItems,
            id,
            Into::<AlignItems>::into(props.cross_axis_alignment) as u32
        );
        push_command!(
            SHARED_BUFFER,
            SetPadding,
            id,
            props.padding.0,
            props.padding.1,
            props.padding.2,
            props.padding.3
        );
        push_command!(SHARED_BUFFER, SetBorderRadius, id, props.corner_radius);
        push_command!(
            SHARED_BUFFER,
            SetClipToBounds,
            id,
            if props.clip_to_bounds { 1u8 } else { 0u8 }
        );

        // Apply size and color via BaseView methods
        let view = if let Some(width) = props.width {
            BaseView::width(view, width)
        } else {
            view
        };
        let view = if let Some(height) = props.height {
            BaseView::height(view, height)
        } else {
            view
        };
        let view = if let Some(color) = props.background {
            BaseView::color(view, color)
        } else {
            view
        };

        view
    }

    #[allow(dead_code)]
    fn ensure_view(&mut self) {
        if self.view.is_none() {
            let view = Self::build_view(&self.props, FlexDirection::Column);
            self.view = Some(view);
        }
    }

    fn rebuild_view(&mut self) {
        // Update existing view if present, otherwise create new
        if let Some(view) = self.view.take() {
            let id = view.node_id();
            select_node(id);

            // Apply flex properties directly via commands
            push_command!(
                SHARED_BUFFER,
                SetFlexDirection,
                id,
                FlexDirection::Column as u32
            );
            push_command!(
                SHARED_BUFFER,
                SetJustifyContent,
                id,
                Into::<JustifyContent>::into(self.props.main_axis_alignment) as u32
            );
            push_command!(
                SHARED_BUFFER,
                SetAlignItems,
                id,
                Into::<AlignItems>::into(self.props.cross_axis_alignment) as u32
            );
            push_command!(
                SHARED_BUFFER,
                SetPadding,
                id,
                self.props.padding.0,
                self.props.padding.1,
                self.props.padding.2,
                self.props.padding.3
            );
            push_command!(SHARED_BUFFER, SetBorderRadius, id, self.props.corner_radius);
            push_command!(
                SHARED_BUFFER,
                SetClipToBounds,
                id,
                if self.props.clip_to_bounds { 1u8 } else { 0u8 }
            );

            // Apply size and color via BaseView methods
            let view = if let Some(width) = self.props.width {
                BaseView::width(view, width)
            } else {
                view
            };
            let view = if let Some(height) = self.props.height {
                BaseView::height(view, height)
            } else {
                view
            };
            let view = if let Some(color) = self.props.background {
                BaseView::color(view, color)
            } else {
                view
            };

            // Re-add all children
            let view = self
                .children
                .iter()
                .fold(view, |v, &child_id| BaseView::child(v, child_id));

            self.view = Some(view);
        } else {
            // Create new view
            let mut view = Self::build_view(&self.props, FlexDirection::Column);
            // Add all children
            for &child_id in &self.children {
                view = BaseView::child(view, child_id);
            }
            self.view = Some(view);
        }
    }

    /// Set spacing between children
    pub fn spacing(mut self, value: f32) -> Self {
        self.props.spacing = value;
        self
    }

    /// Set main axis alignment
    pub fn main_axis_alignment(mut self, alignment: MainAxisAlignment) -> Self {
        self.props.main_axis_alignment = alignment;
        self.rebuild_view();
        self
    }

    /// Set cross axis alignment
    pub fn cross_axis_alignment(mut self, alignment: CrossAxisAlignment) -> Self {
        self.props.cross_axis_alignment = alignment;
        self.rebuild_view();
        self
    }

    /// Set padding
    pub fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.props.padding = padding.into().to_tuple();
        self.rebuild_view();
        self
    }

    /// Set width
    pub fn width(mut self, width: impl Into<SizeUnit>) -> Self {
        self.props.width = Some(width.into());
        self.rebuild_view();
        self
    }

    /// Set height
    pub fn height(mut self, height: impl Into<SizeUnit>) -> Self {
        self.props.height = Some(height.into());
        self.rebuild_view();
        self
    }

    /// Set background color
    pub fn background(mut self, color: impl Into<Color>) -> Self {
        self.props.background = Some(color.into().to_tuple());
        self.rebuild_view();
        self
    }

    /// Set corner radius
    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.props.corner_radius = radius;
        self.rebuild_view();
        self
    }

    /// Enable/disable clipping
    pub fn clip_to_bounds(mut self, clip: bool) -> Self {
        self.props.clip_to_bounds = clip;
        self.rebuild_view();
        self
    }

    // === Flex-specific properties (moved from BaseView) ===

    /// Set flex direction (Column always uses Column direction)
    pub fn flex_direction(self, direction: FlexDirection) -> Self {
        if let Some(ref view) = self.view {
            select_node(view.node_id());
            push_command!(
                SHARED_BUFFER,
                SetFlexDirection,
                view.node_id(),
                direction as u32
            );
        }
        self
    }

    /// Set justify content (main axis alignment)
    pub fn justify_content(self, justify: JustifyContent) -> Self {
        if let Some(ref view) = self.view {
            select_node(view.node_id());
            push_command!(
                SHARED_BUFFER,
                SetJustifyContent,
                view.node_id(),
                justify as u32
            );
        }
        self
    }

    /// Set align items (cross axis alignment)
    pub fn align_items(self, align: AlignItems) -> Self {
        if let Some(ref view) = self.view {
            select_node(view.node_id());
            push_command!(SHARED_BUFFER, SetAlignItems, view.node_id(), align as u32);
        }
        self
    }

    /// Set flex wrap
    pub fn flex_wrap(self, wrap: FlexWrap) -> Self {
        if let Some(ref view) = self.view {
            select_node(view.node_id());
            push_command!(SHARED_BUFFER, SetFlexWrap, view.node_id(), wrap as u32);
        }
        self
    }

    /// Set align content (for multi-line flex containers)
    pub fn align_content(self, align: crate::AlignContent) -> Self {
        if let Some(ref view) = self.view {
            select_node(view.node_id());
            push_command!(SHARED_BUFFER, SetAlignContent, view.node_id(), align as u32);
        }
        self
    }

    /// Add a child view
    pub fn child(mut self, child: impl BaseView) -> Self {
        let child_id = child.node_id();
        self.children.push(child_id);
        // Build view if needed and add child
        if let Some(view) = self.view.take() {
            self.view = Some(BaseView::child(view, child_id));
        } else {
            self.rebuild_view();
        }
        self
    }

    /// Add a child with spacing (applies top margin)
    pub fn spaced_child(self, child: impl BaseView, _spacing: f32) -> Self {
        // For now, just add the child. In the future, we could wrap it
        // with a spacer view
        self.child(child)
    }
}

impl BaseView for Column {
    fn node_id(&self) -> u32 {
        self.view
            .as_ref()
            .map(|v| v.node_id())
            .unwrap_or_else(|| Self::build_view(&self.props, FlexDirection::Column).node_id())
    }
}

impl Default for Column {
    fn default() -> Self {
        Self::new()
    }
}

/// Row - horizontal flex container
///
/// # Example
/// ```rust,ignore
/// Row::new()
///     .spacing(8.0)
///     .main_axis_alignment(MainAxisAlignment::SpaceBetween)
///     .child(Button::new("Cancel"))
///     .child(Button::new("OK"))
/// ```
pub struct Row {
    view: Option<View>,
    props: FlexContainerProps,
    children: Vec<u32>,
}

impl Row {
    pub fn new() -> Self {
        let props = FlexContainerProps::default();
        Self {
            view: None,
            props,
            children: Vec::new(),
        }
    }

    fn build_view(props: &FlexContainerProps, direction: FlexDirection) -> View {
        let view = View::new();
        let id = view.node_id();

        // Apply flex properties directly via commands
        select_node(id);
        push_command!(SHARED_BUFFER, SetFlexDirection, id, direction as u32);
        push_command!(
            SHARED_BUFFER,
            SetJustifyContent,
            id,
            Into::<JustifyContent>::into(props.main_axis_alignment) as u32
        );
        push_command!(
            SHARED_BUFFER,
            SetAlignItems,
            id,
            Into::<AlignItems>::into(props.cross_axis_alignment) as u32
        );
        push_command!(
            SHARED_BUFFER,
            SetPadding,
            id,
            props.padding.0,
            props.padding.1,
            props.padding.2,
            props.padding.3
        );
        push_command!(SHARED_BUFFER, SetBorderRadius, id, props.corner_radius);
        push_command!(
            SHARED_BUFFER,
            SetClipToBounds,
            id,
            if props.clip_to_bounds { 1u8 } else { 0u8 }
        );

        // Apply size and color via BaseView methods
        let view = if let Some(width) = props.width {
            BaseView::width(view, width)
        } else {
            view
        };
        let view = if let Some(height) = props.height {
            BaseView::height(view, height)
        } else {
            view
        };
        let view = if let Some(color) = props.background {
            BaseView::color(view, color)
        } else {
            view
        };

        view
    }

    #[allow(dead_code)]
    fn ensure_view(&mut self) {
        if self.view.is_none() {
            let view = Self::build_view(&self.props, FlexDirection::Row);
            self.view = Some(view);
        }
    }

    fn rebuild_view(&mut self) {
        // Update existing view if present, otherwise create new
        if let Some(view) = self.view.take() {
            let id = view.node_id();
            select_node(id);

            // Apply flex properties directly via commands
            push_command!(
                SHARED_BUFFER,
                SetFlexDirection,
                id,
                FlexDirection::Row as u32
            );
            push_command!(
                SHARED_BUFFER,
                SetJustifyContent,
                id,
                Into::<JustifyContent>::into(self.props.main_axis_alignment) as u32
            );
            push_command!(
                SHARED_BUFFER,
                SetAlignItems,
                id,
                Into::<AlignItems>::into(self.props.cross_axis_alignment) as u32
            );
            push_command!(
                SHARED_BUFFER,
                SetPadding,
                id,
                self.props.padding.0,
                self.props.padding.1,
                self.props.padding.2,
                self.props.padding.3
            );
            push_command!(SHARED_BUFFER, SetBorderRadius, id, self.props.corner_radius);
            push_command!(
                SHARED_BUFFER,
                SetClipToBounds,
                id,
                if self.props.clip_to_bounds { 1u8 } else { 0u8 }
            );

            // Apply size and color via BaseView methods
            let view = if let Some(width) = self.props.width {
                BaseView::width(view, width)
            } else {
                view
            };
            let view = if let Some(height) = self.props.height {
                BaseView::height(view, height)
            } else {
                view
            };
            let view = if let Some(color) = self.props.background {
                BaseView::color(view, color)
            } else {
                view
            };

            let view = self
                .children
                .iter()
                .fold(view, |v, &child_id| BaseView::child(v, child_id));
            self.view = Some(view);
        } else {
            let mut view = Self::build_view(&self.props, FlexDirection::Row);
            for &child_id in &self.children {
                view = BaseView::child(view, child_id);
            }
            self.view = Some(view);
        }
    }

    /// Set spacing between children
    pub fn spacing(mut self, value: f32) -> Self {
        self.props.spacing = value;
        self
    }

    /// Set main axis alignment
    pub fn main_axis_alignment(mut self, alignment: MainAxisAlignment) -> Self {
        self.props.main_axis_alignment = alignment;
        self.rebuild_view();
        self
    }

    /// Set cross axis alignment
    pub fn cross_axis_alignment(mut self, alignment: CrossAxisAlignment) -> Self {
        self.props.cross_axis_alignment = alignment;
        self.rebuild_view();
        self
    }

    /// Set padding
    pub fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.props.padding = padding.into().to_tuple();
        self.rebuild_view();
        self
    }

    /// Set width
    pub fn width(mut self, width: impl Into<SizeUnit>) -> Self {
        self.props.width = Some(width.into());
        self.rebuild_view();
        self
    }

    /// Set height
    pub fn height(mut self, height: impl Into<SizeUnit>) -> Self {
        self.props.height = Some(height.into());
        self.rebuild_view();
        self
    }

    /// Set background color
    pub fn background(mut self, color: impl Into<Color>) -> Self {
        self.props.background = Some(color.into().to_tuple());
        self.rebuild_view();
        self
    }

    /// Set corner radius
    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.props.corner_radius = radius;
        self.rebuild_view();
        self
    }

    /// Enable/disable clipping
    pub fn clip_to_bounds(mut self, clip: bool) -> Self {
        self.props.clip_to_bounds = clip;
        self.rebuild_view();
        self
    }

    // === Flex-specific properties (moved from BaseView) ===

    /// Set flex direction (Row always uses Row direction)
    pub fn flex_direction(self, direction: FlexDirection) -> Self {
        if let Some(ref view) = self.view {
            select_node(view.node_id());
            push_command!(
                SHARED_BUFFER,
                SetFlexDirection,
                view.node_id(),
                direction as u32
            );
        }
        self
    }

    /// Set justify content (main axis alignment)
    pub fn justify_content(self, justify: JustifyContent) -> Self {
        if let Some(ref view) = self.view {
            select_node(view.node_id());
            push_command!(
                SHARED_BUFFER,
                SetJustifyContent,
                view.node_id(),
                justify as u32
            );
        }
        self
    }

    /// Set align items (cross axis alignment)
    pub fn align_items(self, align: AlignItems) -> Self {
        if let Some(ref view) = self.view {
            select_node(view.node_id());
            push_command!(SHARED_BUFFER, SetAlignItems, view.node_id(), align as u32);
        }
        self
    }

    /// Set flex wrap
    pub fn flex_wrap(self, wrap: FlexWrap) -> Self {
        if let Some(ref view) = self.view {
            select_node(view.node_id());
            push_command!(SHARED_BUFFER, SetFlexWrap, view.node_id(), wrap as u32);
        }
        self
    }

    /// Set align content (for multi-line flex containers)
    pub fn align_content(self, align: crate::AlignContent) -> Self {
        if let Some(ref view) = self.view {
            select_node(view.node_id());
            push_command!(SHARED_BUFFER, SetAlignContent, view.node_id(), align as u32);
        }
        self
    }

    /// Add a child view
    pub fn child(mut self, child: impl BaseView) -> Self {
        let child_id = child.node_id();
        self.children.push(child_id);
        // Build view if needed and add child
        if let Some(view) = self.view.take() {
            self.view = Some(BaseView::child(view, child_id));
        } else {
            self.rebuild_view();
        }
        self
    }

    /// Add a child with spacing (applies left margin)
    pub fn spaced_child(self, child: impl BaseView, _spacing: f32) -> Self {
        self.child(child)
    }
}

impl BaseView for Row {
    fn node_id(&self) -> u32 {
        self.view
            .as_ref()
            .map(|v| v.node_id())
            .unwrap_or_else(|| Self::build_view(&self.props, FlexDirection::Row).node_id())
    }
}

impl Default for Row {
    fn default() -> Self {
        Self::new()
    }
}

/// Spacer - creates flexible empty space
///
/// # Example
/// ```rust,ignore
/// Row::new()
///     .child(Text("Left"))
///     .child(Spacer::new())  // Pushes "Right" to the end
///     .child(Text("Right"))
/// ```
pub struct Spacer {
    view: View,
}

impl Spacer {
    pub fn new() -> Self {
        let view = View::new().flex_grow(1.0);
        Self { view }
    }

    pub fn flex(mut self, flex: f32) -> Self {
        self.view = View::new().flex_grow(flex);
        self
    }
}

impl BaseView for Spacer {
    fn node_id(&self) -> u32 {
        self.view.node_id()
    }
}

impl Default for Spacer {
    fn default() -> Self {
        Self::new()
    }
}

/// Divider - horizontal or vertical line separator
///
/// # Example
/// ```rust,ignore
/// Column::new()
///     .child(Text("Section 1"))
///     .child(Divider::new())
///     .child(Text("Section 2"))
/// ```
pub struct Divider {
    view: View,
}

impl Divider {
    pub fn new() -> Self {
        let view = View::new()
            .width(SizeUnit::Percent(100.0))
            .height(SizeUnit::Px(1.0))
            .color((200, 200, 200, 255));
        Self { view }
    }

    /// Create a vertical divider
    pub fn vertical() -> Self {
        let view = View::new()
            .width(SizeUnit::Px(1.0))
            .height(SizeUnit::Percent(100.0))
            .color((200, 200, 200, 255));
        Self { view }
    }

    pub fn color(mut self, color: impl Into<Color>) -> Self {
        self.view = View::new()
            .width(SizeUnit::Percent(100.0))
            .height(SizeUnit::Px(1.0))
            .color(color.into().to_tuple());
        self
    }

    pub fn thickness(mut self, thickness: f32) -> Self {
        self.view = View::new()
            .width(SizeUnit::Percent(100.0))
            .height(SizeUnit::Px(thickness))
            .color((200, 200, 200, 255));
        self
    }
}

impl BaseView for Divider {
    fn node_id(&self) -> u32 {
        self.view.node_id()
    }
}

impl Default for Divider {
    fn default() -> Self {
        Self::new()
    }
}

// ===== Helper types =====

/// Padding specification
pub struct Padding {
    top: f32,
    right: f32,
    bottom: f32,
    left: f32,
}

impl Padding {
    pub fn all(value: f32) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }

    pub fn symmetric(horizontal: f32, vertical: f32) -> Self {
        Self {
            top: vertical,
            right: horizontal,
            bottom: vertical,
            left: horizontal,
        }
    }

    pub fn only(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }

    fn to_tuple(self) -> (f32, f32, f32, f32) {
        (self.top, self.right, self.bottom, self.left)
    }
}

impl From<f32> for Padding {
    fn from(value: f32) -> Self {
        Self::all(value)
    }
}

impl From<(f32, f32, f32, f32)> for Padding {
    fn from(tuple: (f32, f32, f32, f32)) -> Self {
        Self {
            top: tuple.0,
            right: tuple.1,
            bottom: tuple.2,
            left: tuple.3,
        }
    }
}

/// Color specification
pub struct Color {
    r: u8,
    g: u8,
    b: u8,
    a: u8,
}

impl Color {
    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    fn to_tuple(self) -> (u32, u32, u32, u32) {
        (self.r as u32, self.g as u32, self.b as u32, self.a as u32)
    }
}

impl From<(u8, u8, u8, u8)> for Color {
    fn from(tuple: (u8, u8, u8, u8)) -> Self {
        Self {
            r: tuple.0,
            g: tuple.1,
            b: tuple.2,
            a: tuple.3,
        }
    }
}

impl From<(u32, u32, u32, u32)> for Color {
    fn from(tuple: (u32, u32, u32, u32)) -> Self {
        Self {
            r: tuple.0 as u8,
            g: tuple.1 as u8,
            b: tuple.2 as u8,
            a: tuple.3 as u8,
        }
    }
}

// SizeUnit is already exported from crate root
