// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Test-only archive for renderer experiments that are not wired into the
//! production backend path.
//!
//! Keeping these modules under `#[cfg(test)]` makes their unit tests and type
//! checks run with `cargo test --lib`, while avoiding ambiguity about whether
//! they participate in the runtime renderer architecture.

#![allow(dead_code)]

mod composite_pipeline;
mod layer;
