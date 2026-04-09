// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::state::SharedState;
use dyxel_render_api::{BackendConfig, DeviceHandle, QueueHandle, RenderBackend, RenderContext};
use dyxel_render_api::{SharedMutex, SharedPtr};
use dyxel_render_vello::VelloBackend;

// std::sync::Mutex is used for wasm3 function handles (separate from SharedMutex)
#[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))]
use std::sync::Mutex;

pub struct LogicState {
    pub shared_state: SharedPtr<SharedMutex<SharedState>>,
    #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))]
    pub _env: wasm3::Environment,
    #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))]
    pub _rt: wasm3::Runtime,
    #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))]
    pub tick_fn: Mutex<Option<wasm3::Function<'static, (), ()>>>,
    #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))]
    pub on_click_fn: Mutex<Option<wasm3::Function<'static, (u32,), ()>>>,
    #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))]
    pub shared_buffer_ptr: Mutex<Option<u32>>,
}

pub struct RenderState {
    pub context: RenderContext,
    pub backend: Box<dyn RenderBackend>,
    pub shared_state: SharedPtr<SharedMutex<SharedState>>,
}

// LogicState contains Wasm3 components which are NOT Sync.
// We only need it to be Send so it can be moved to the Logic Thread.
unsafe impl Send for LogicState {}

unsafe impl Send for RenderState {}
unsafe impl Sync for RenderState {}

impl RenderState {
    pub fn on_lifecycle_event(&self, event: dyxel_render_api::LifecycleEvent) {
        self.backend.on_lifecycle_event(event);
    }

    /// Enable performance overlay display
    pub fn enable_perf_overlay(&self) {
        // Cast to VelloBackend and enable overlay directly
        if let Some(vello_backend) = self.backend.as_any().downcast_ref::<VelloBackend>() {
            vello_backend.enable_perf_overlay();
        }
    }
}

pub async fn setup_engine(ddir: String) -> anyhow::Result<(LogicState, RenderState)> {
    // Create Vello-specific render context
    let mut v_context = vello::util::RenderContext::new();

    let dev_id = pollster::block_on(async { v_context.device(None).await })
        .ok_or_else(|| anyhow::anyhow!("No device found"))?;

    let device = &v_context.devices[dev_id].device;
    let queue = &v_context.devices[dev_id].queue;

    let backend = VelloBackend::new();
    // Wrap device and queue in handles for the abstract API
    let device_handle = DeviceHandle::new(device);
    let queue_handle = QueueHandle::new(queue);
    backend.init(
        device_handle,
        queue_handle,
        BackendConfig { data_dir: ddir },
    )?;

    // Wrap the Vello context in the abstract RenderContext
    let context = RenderContext::new(v_context);

    let shared_state = SharedPtr::new(SharedMutex::new(SharedState::new()));

    #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))]
    let (env, rt) = {
        let env =
            wasm3::Environment::new().map_err(|e| anyhow::anyhow!("Environment failed: {}", e))?;
        let rt = env
            .create_runtime(1024 * 2048)
            .map_err(|e| anyhow::anyhow!("Runtime failed: {}", e))?;
        (env, rt)
    };

    let logic = LogicState {
        shared_state: shared_state.clone(),
        #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))]
        _env: env,
        #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))]
        _rt: rt,
        #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))]
        tick_fn: Mutex::new(None),
        #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))]
        on_click_fn: Mutex::new(None),
        #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))]
        shared_buffer_ptr: Mutex::new(None),
    };

    let render = RenderState {
        context,
        backend: Box::new(backend),
        shared_state,
    };

    Ok((logic, render))
}

#[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))]
impl LogicState {
    pub fn load_wasm(&mut self, wasm_path: String) -> anyhow::Result<()> {
        // Clear shared state before loading new WASM
        {
            let mut state = self.shared_state.lock().unwrap();
            state.clear();
        }

        // Recreate WASM runtime to ensure clean state for hot restart
        // This is necessary because WASM static variables persist across module reloads
        {
            let new_env = wasm3::Environment::new()
                .map_err(|e| anyhow::anyhow!("Environment failed: {}", e))?;
            let new_rt = new_env
                .create_runtime(1024 * 2048)
                .map_err(|e| anyhow::anyhow!("Runtime failed: {}", e))?;
            self._env = new_env;
            self._rt = new_rt;
        }

        let wasm_bytes = std::fs::read(&wasm_path)
            .map_err(|e| anyhow::anyhow!("Failed to read WASM at {}: {}", wasm_path, e))?;

        use crate::runtime::process_commands;

        let parsed = self
            ._env
            .parse_module(wasm_bytes)
            .map_err(|e| anyhow::anyhow!("Parse failed: {}", e))?;
        let mut module = self
            ._rt
            .load_module(parsed)
            .map_err(|e| anyhow::anyhow!("Load failed: {}", e))?;

        let bptr = module
            .find_function::<(), u32>("dyxel_get_shared_buffer_ptr")
            .map_err(|e| anyhow::anyhow!("Func not found: {}", e))?
            .call()
            .map_err(|e| anyhow::anyhow!("Call failed: {}", e))?;

        let s_inner = self.shared_state.clone();
        let _ = module.link_closure("env", "ui_force_layout", move |ctx, ()| {
            let mem = unsafe { &mut *ctx.memory_mut() };
            let _ = crate::runtime::process_commands(mem, bptr, &s_inner);
            Ok(())
        });

        // Link console_log for WASM debugging
        let _ = module.link_closure("env", "console_log", move |ctx, (ptr, len): (u32, u32)| {
            let mem = unsafe { &*ctx.memory() };
            let start = ptr as usize;
            let end = start + len as usize;
            if end <= mem.len() {
                let msg = String::from_utf8_lossy(&mem[start..end]);
                log::info!("[WASM] {}", msg);
            }
            Ok(())
        });

        let main_fn = module
            .find_function::<(), ()>("main")
            .or_else(|_| module.find_function::<(), ()>("_main"))
            .map_err(|_| anyhow::anyhow!("Main not found"))?;
        let get_hash_fn = module
            .find_function::<(), u64>("dyxel_get_protocol_hash")
            .map_err(|_| anyhow::anyhow!("dyxel_get_protocol_hash not found"))?;

        let guest_hash = get_hash_fn
            .call()
            .map_err(|_| anyhow::anyhow!("Failed to call get_hash"))?;
        if guest_hash != dyxel_shared::PROTOCOL_HASH {
            return Err(anyhow::anyhow!(
                "Protocol mismatch! Host: {}, Guest: {}",
                dyxel_shared::PROTOCOL_HASH,
                guest_hash
            ));
        }

        let tick_fn = module
            .find_function::<(), ()>("guest_tick")
            .or_else(|_| module.find_function::<(), ()>("vello_tick"))
            .or_else(|_| module.find_function::<(), ()>("_guest_tick"))
            .map_err(|_| anyhow::anyhow!("Tick func not found"))?;
        let on_click_fn = module
            .find_function::<(u32,), ()>("on_node_click")
            .or_else(|_| module.find_function::<(u32,), ()>("_on_node_click"))
            .map_err(|_| anyhow::anyhow!("OnClick func not found"))?;

        let _ = main_fn.call();
        let memory = unsafe { &mut *self._rt.memory_mut() };

        // Debug: check command length before processing
        let bs = bptr as usize;
        log::info!("Shared buffer ptr: {}, memory size: {}", bptr, memory.len());
        let clen = u32::from_le_bytes(memory[bs..bs + 4].try_into().unwrap_or([0, 0, 0, 0]));
        log::info!(
            "After main(): command_len = {} (raw bytes: {:?})",
            clen,
            &memory[bs..bs + 4]
        );

        let result = process_commands(memory, bptr, &self.shared_state);
        if let Err(e) = &result {
            log::error!("process_commands failed: {}", e);
        }

        *self.tick_fn.lock().unwrap() = Some(unsafe { std::mem::transmute(tick_fn) });
        *self.on_click_fn.lock().unwrap() = Some(unsafe { std::mem::transmute(on_click_fn) });
        *self.shared_buffer_ptr.lock().unwrap() = Some(bptr);

        Ok(())
    }
}
