// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use dyxel_render_api::{RenderBackend, RenderContext, BackendConfig};
use dyxel_render_vello::VelloBackend;
use crate::state::SharedState;

// Platform-specific synchronization primitives
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Arc, Mutex};
#[cfg(target_arch = "wasm32")]
use std::rc::Rc;
#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;

#[cfg(not(target_arch = "wasm32"))]
pub type SharedPtr<T> = Arc<T>;
#[cfg(target_arch = "wasm32")]
pub type SharedPtr<T> = Rc<T>;

#[cfg(not(target_arch = "wasm32"))]
pub type SharedMutex<T> = Mutex<T>;
#[cfg(target_arch = "wasm32")]
pub type SharedMutex<T> = RefCell<T>;

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

pub struct LogicState {
    pub shared_state: SharedPtr<SharedMutex<SharedState>>,
    #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))] pub _env: wasm3::Environment,
    #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))] pub _rt: wasm3::Runtime,
    #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))] pub tick_fn: Mutex<Option<wasm3::Function<'static, (), ()>>>,
    #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))] pub on_click_fn: Mutex<Option<wasm3::Function<'static, (u32,), ()>>>,
    #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))] pub shared_buffer_ptr: Mutex<Option<u32>>,
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
        // Cast to VelloBackend to enable overlay
        // This is a bit of a hack but works for now
        if let Some(vello_backend) = self.backend.as_any().downcast_ref::<VelloBackend>() {
            vello_backend.enable_perf_overlay();
        }
    }
}

pub async fn setup_engine(ddir: String) -> anyhow::Result<(LogicState, RenderState)> {
    let setup_start = Instant::now();
    
    let mut context = RenderContext::new(); 
    
    let dev_id = pollster::block_on(async {
        context.device(None).await
    }).ok_or_else(|| anyhow::anyhow!("No device found"))?;
    
    let device = &context.devices[dev_id].device;
    let queue = &context.devices[dev_id].queue;
    log::info!("[Perf] setup_engine: WGPU device ready in {:?}", setup_start.elapsed());

    let backend = VelloBackend::new();
    let init_start = Instant::now();
    backend.init(device, queue, BackendConfig { data_dir: ddir })?;
    log::info!("[Perf] setup_engine: VelloBackend init took {:?}", init_start.elapsed());

    let shared_state = SharedPtr::new(SharedMutex::new(SharedState::new()));

    #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))]
    let (env, rt) = {
        let wasm3_start = Instant::now();
        let env = wasm3::Environment::new().map_err(|e| anyhow::anyhow!("Environment failed: {}", e))?; 
        let rt = env.create_runtime(1024 * 2048).map_err(|e| anyhow::anyhow!("Runtime failed: {}", e))?;
        log::info!("[Perf] setup_engine: WASM3 env+runtime creation took {:?}", wasm3_start.elapsed());
        (env, rt)
    };

    let logic = LogicState { 
        shared_state: shared_state.clone(),
        #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))] _env: env, 
        #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))] _rt: rt, 
        #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))] tick_fn: Mutex::new(None), 
        #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))] on_click_fn: Mutex::new(None), 
        #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))] shared_buffer_ptr: Mutex::new(None), 
    };

    let render = RenderState {
        context,
        backend: Box::new(backend),
        shared_state,
    };

    log::info!("[Perf] setup_engine: Total time {:?}", setup_start.elapsed());
    Ok((logic, render))
}

#[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))]
impl LogicState {
    pub fn load_wasm(&mut self, wasm_path: String) -> anyhow::Result<()> {
        #[cfg(not(target_arch = "wasm32"))]
        let wasm_init_start = Instant::now();
        
        // Clear shared state before loading new WASM
        {
            let t = Instant::now();
            let mut state = self.shared_state.lock().unwrap();
            state.clear();
            log::info!("[Perf] load_wasm: Shared state cleared in {:?}", t.elapsed());
        }
        
        // Recreate WASM runtime to ensure clean state for hot restart
        // This is necessary because WASM static variables persist across module reloads
        {
            let t = Instant::now();
            let new_env = wasm3::Environment::new().map_err(|e| anyhow::anyhow!("Environment failed: {}", e))?;
            let new_rt = new_env.create_runtime(1024 * 2048).map_err(|e| anyhow::anyhow!("Runtime failed: {}", e))?;
            self._env = new_env;
            self._rt = new_rt;
            log::info!("[Perf] load_wasm: WASM runtime recreated in {:?}", t.elapsed());
        }
        
        let t = Instant::now();
        let wasm_bytes = std::fs::read(&wasm_path).map_err(|e| anyhow::anyhow!("Failed to read WASM at {}: {}", wasm_path, e))?;
        log::info!("[Perf] load_wasm: Read {} bytes from disk in {:?}", wasm_bytes.len(), t.elapsed());
        
        use crate::runtime::process_commands;
        
        let t = Instant::now();
        let parsed = self._env.parse_module(wasm_bytes).map_err(|e| anyhow::anyhow!("Parse failed: {}", e))?;
        let parse_elapsed = t.elapsed();
        
        let t = Instant::now();
        let mut module = self._rt.load_module(parsed).map_err(|e| anyhow::anyhow!("Load failed: {}", e))?;
        let load_elapsed = t.elapsed();
        log::info!("[Perf] load_wasm: Parse module {:?}, Load module {:?}", parse_elapsed, load_elapsed);
        
        let t = Instant::now();
        let bptr = module.find_function::<(), u32>("dyxel_get_shared_buffer_ptr").map_err(|e| anyhow::anyhow!("Func not found: {}", e))?.call().map_err(|e| anyhow::anyhow!("Call failed: {}", e))?;
        log::info!("[Perf] load_wasm: Get shared buffer ptr in {:?}", t.elapsed());

        let s_inner = self.shared_state.clone();
        let _ = module.link_closure("env", "ui_force_layout", move |ctx, ()| { 
            let mem = unsafe { &mut *ctx.memory_mut() }; 
            let _ = crate::runtime::process_commands(mem, bptr, &s_inner); 
            Ok(()) 
        });

        let t = Instant::now();
        let main_fn = module.find_function::<(), ()>("main").or_else(|_| module.find_function::<(), ()>("_main")).map_err(|_| anyhow::anyhow!("Main not found"))?;
        let get_hash_fn = module.find_function::<(), u64>("dyxel_get_protocol_hash").map_err(|_| anyhow::anyhow!("dyxel_get_protocol_hash not found"))?;

        let guest_hash = get_hash_fn.call().map_err(|_| anyhow::anyhow!("Failed to call get_hash"))?;
        if guest_hash != dyxel_shared::PROTOCOL_HASH { 
            return Err(anyhow::anyhow!("Protocol mismatch! Host: {}, Guest: {}", dyxel_shared::PROTOCOL_HASH, guest_hash)); 
        }
        log::info!("[Perf] load_wasm: Find functions and check hash in {:?}", t.elapsed());
        
        let tick_fn = module.find_function::<(), ()>("guest_tick").or_else(|_| module.find_function::<(), ()>("vello_tick")).or_else(|_| module.find_function::<(), ()>("_guest_tick")).map_err(|_| anyhow::anyhow!("Tick func not found"))?;
        let on_click_fn = module.find_function::<(u32,), ()>("on_node_click").or_else(|_| module.find_function::<(u32,), ()>("_on_node_click")).map_err(|_| anyhow::anyhow!("OnClick func not found"))?;
        
        let t = Instant::now();
        let _ = main_fn.call(); 
        let main_elapsed = t.elapsed();
        
        let t = Instant::now();
        let memory = unsafe { &mut *self._rt.memory_mut() }; 
        let _ = process_commands(memory, bptr, &self.shared_state);
        let process_elapsed = t.elapsed();
        log::info!("[Perf] load_wasm: WASM main() took {:?}, process_commands took {:?}", main_elapsed, process_elapsed);
        
        #[cfg(not(target_arch = "wasm32"))]
        log::info!("[Perf] load_wasm: Total time {:?}", wasm_init_start.elapsed());

        *self.tick_fn.lock().unwrap() = Some(unsafe { std::mem::transmute(tick_fn) });
        *self.on_click_fn.lock().unwrap() = Some(unsafe { std::mem::transmute(on_click_fn) });
        *self.shared_buffer_ptr.lock().unwrap() = Some(bptr);

        Ok(())
    }
}
