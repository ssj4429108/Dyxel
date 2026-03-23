// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::engine::EngineState;
use dyxel_render_api::SurfaceState;

pub fn render_frame(e: &mut EngineState, s: &mut dyn SurfaceState) {
    let device = &e.context.devices[0].device;
    let queue = &e.context.devices[0].queue;
    
    if let Err(err) = e.backend.render(device, queue, s, &e.shared_state) {
        log::error!("Render error: {:?}", err);
    }
}
