// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Dyxel Sample Demos
//!
//! Available demos:
//! - dual_track_1000_demo: 1000 nodes stress test with paging
//! - input_proxy_demo: Gesture recognition and input validation

// Demo modules
mod dual_track_1000_demo;
mod input_proxy_demo;

// Select which demo to run
// Change this to switch between demos
// use dual_track_1000_demo as CurrentDemo;
use input_proxy_demo as CurrentDemo;

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn main() {
    CurrentDemo::init();
}

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn guest_tick() {
    CurrentDemo::tick();
}
