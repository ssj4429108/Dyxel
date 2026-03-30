// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Week 4 Dual-Track Demo: 1000 Nodes with Paging

mod dual_track_1000_demo;
use dual_track_1000_demo as demo;

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn main() {
    demo::init();
}

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn guest_tick() {
    demo::tick();
}
