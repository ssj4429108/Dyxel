// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use serde::{Deserialize, Serialize};

#[repr(u32)]
#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub enum FlexDirection {
    Row = 0,
    Column = 1,
    RowReverse = 2,
    ColumnReverse = 3,
}

#[repr(u32)]
#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub enum JustifyContent {
    FlexStart = 0,
    Center = 1,
    FlexEnd = 2,
    SpaceBetween = 3,
    SpaceAround = 4,
    SpaceEvenly = 5,
}

#[repr(u32)]
#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub enum AlignItems {
    FlexStart = 0,
    Center = 1,
    FlexEnd = 2,
    Stretch = 3,
}

#[repr(u32)]
#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub enum FlexWrap {
    NoWrap = 0,
    Wrap = 1,
    WrapReverse = 2,
}

#[repr(u32)]
#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub enum AlignContent {
    FlexStart = 0,
    Center = 1,
    FlexEnd = 2,
    Stretch = 3,
    SpaceBetween = 4,
    SpaceAround = 5,
    SpaceEvenly = 6,
}

#[repr(u32)]
#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub enum PositionType {
    Relative = 0,
    Absolute = 1,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub enum Dimension {
    Auto,
    Pixels(f32),
    Percent(f32),
}

impl From<&str> for Dimension {
    fn from(s: &str) -> Self {
        if s == "auto" {
            Dimension::Auto
        } else if s.ends_with('%') {
            Dimension::Percent(s[..s.len() - 1].parse().unwrap_or(0.0))
        } else {
            Dimension::Pixels(s.parse().unwrap_or(0.0))
        }
    }
}
impl From<f32> for Dimension {
    fn from(f: f32) -> Self {
        Dimension::Pixels(f)
    }
}
impl From<i32> for Dimension {
    fn from(i: i32) -> Self {
        Dimension::Pixels(i as f32)
    }
}

#[repr(u32)]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    None = 0,
    Button = 1,
    Heading = 2,
    Link = 3,
    Label = 4,
}

#[repr(u32)]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewType {
    Container = 0,
    Text = 1,
    Button = 2,
    Image = 3,
    Input = 4,
}
