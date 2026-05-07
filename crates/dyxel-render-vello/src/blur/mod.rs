// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Backdrop blur data types and pure helper logic.
//!
//! Higher-level pass orchestration still lives in `lib.rs` during the first
//! refactor phase. This module intentionally starts with pure data/helpers so
//! the move is behavior-preserving.

pub(crate) mod atlas;
pub(crate) mod atlas_pass;
pub(crate) mod children;
pub(crate) mod composite;
pub(crate) mod dirty;
pub(crate) mod entry;
pub(crate) mod passes;
pub(crate) mod pipeline;
pub(crate) mod types;

pub(crate) use atlas::*;
pub(crate) use atlas_pass::*;
pub(crate) use children::*;
pub(crate) use composite::*;
pub(crate) use dirty::*;
pub(crate) use entry::*;
pub(crate) use passes::*;
pub(crate) use types::*;
