// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::engine::RenderState;
use dyxel_render_api::{SurfaceState, DeviceHandle, QueueHandle};

pub fn render_frame(e: &mut RenderState, s: &mut dyn SurfaceState) {
    // Downcast context to get Vello-specific types
    // Note: For a truly backend-agnostic approach, we would need to store
    // device and queue handles in RenderState. For now, we downcast since
    // we know we're using Vello.
    if let Some(v_ctx) = e.context.downcast_ref::<vello::util::RenderContext>() {
        let device = &v_ctx.devices[0].device;
        let queue = &v_ctx.devices[0].queue;
        
        // Create handles for the abstract API
        let device_handle = DeviceHandle::new(device);
        let queue_handle = QueueHandle::new(queue);
        
        log::trace!("renderer: Starting frame render, surface size: {}x{}", s.width(), s.height());
        if let Err(err) = e.backend.render(device_handle, queue_handle, s, &e.shared_state) {
            log::error!("renderer: Render error: {:?}", err);
        }
    } else {
        log::error!("renderer: Failed to downcast RenderContext to Vello context");
    }
}
