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

pub struct EngineState { 
    pub context: RenderContext, 
    pub backend: Box<dyn RenderBackend>,
    pub shared_state: SharedPtr<SharedMutex<SharedState>>,
    #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))] pub _env: wasm3::Environment,
    #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))] pub _rt: wasm3::Runtime,
    #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))] pub tick_fn: Mutex<Option<wasm3::Function<'static, (), ()>>>,
    #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))] pub on_click_fn: Mutex<Option<wasm3::Function<'static, (u32,), ()>>>,
    #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))] pub shared_buffer_ptr: Mutex<Option<u32>>,
}

unsafe impl Send for EngineState {}
unsafe impl Sync for EngineState {}

impl EngineState {
    pub fn on_lifecycle_event(&self, event: dyxel_render_api::LifecycleEvent) {
        self.backend.on_lifecycle_event(event);
    }
}

pub async fn setup_engine(ddir: String, _es: SharedPtr<SharedMutex<Option<EngineState>>>) -> anyhow::Result<EngineState> {
    let mut context = RenderContext::new(); 
    
    // 使用 pollster 阻塞等待设备初始化，避免异步调度问题
    let dev_id = pollster::block_on(async {
        context.device(None).await
    }).ok_or_else(|| anyhow::anyhow!("No device found"))?;
    
    let device = &context.devices[dev_id].device;
    let queue = &context.devices[dev_id].queue;

    let backend = VelloBackend::new();
    backend.init(device, queue, BackendConfig { data_dir: ddir })?;

    let shared_state = SharedPtr::new(SharedMutex::new(SharedState::new()));

    #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))]
    let env = wasm3::Environment::new().map_err(|e| anyhow::anyhow!("Environment failed: {}", e))?; 
    #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))]
    let rt = env.create_runtime(1024 * 2048).map_err(|e| anyhow::anyhow!("Runtime failed: {}", e))?;

    let engine = EngineState { 
        context, 
        backend: Box::new(backend),
        shared_state, 
        #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))] _env: env, 
        #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))] _rt: rt, 
        #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))] tick_fn: Mutex::new(None), 
        #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))] on_click_fn: Mutex::new(None), 
        #[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))] shared_buffer_ptr: Mutex::new(None), 
    };

    Ok(engine)
}

#[cfg(all(feature = "wasm3-support", not(target_arch = "wasm32")))]
impl EngineState {
    pub fn load_wasm(&self, wasm_path: String) -> anyhow::Result<()> {
        #[cfg(not(target_arch = "wasm32"))]
        let wasm_init_start = Instant::now();
        let wasm_bytes = std::fs::read(&wasm_path).map_err(|e| anyhow::anyhow!("Failed to read WASM at {}: {}", wasm_path, e))?;
        log::info!("Engine: Using WASM from {} ({} bytes)", wasm_path, wasm_bytes.len());
        
        use crate::runtime::process_commands;
        
        let mut module = self._rt.load_module(self._env.parse_module(wasm_bytes).map_err(|e| anyhow::anyhow!("Parse failed: {}", e))?).map_err(|e| anyhow::anyhow!("Load failed: {}", e))?;
        let bptr = module.find_function::<(), u32>("dyxel_get_shared_buffer_ptr").map_err(|e| anyhow::anyhow!("Func not found: {}", e))?.call().map_err(|e| anyhow::anyhow!("Call failed: {}", e))?;

        let s_inner = self.shared_state.clone();
        let _ = module.link_closure("env", "ui_force_layout", move |ctx, ()| { 
            let mem = unsafe { &mut *ctx.memory_mut() }; 
            let _ = crate::runtime::process_commands(mem, bptr, &s_inner); 
            Ok(()) 
        });

        let main_fn = module.find_function::<(), ()>("main").or_else(|_| module.find_function::<(), ()>("_main")).map_err(|_| anyhow::anyhow!("Main not found"))?;
        let get_hash_fn = module.find_function::<(), u64>("dyxel_get_protocol_hash").map_err(|_| anyhow::anyhow!("dyxel_get_protocol_hash not found"))?;

        let guest_hash = get_hash_fn.call().map_err(|_| anyhow::anyhow!("Failed to call get_hash"))?;
        if guest_hash != dyxel_shared::PROTOCOL_HASH { 
            return Err(anyhow::anyhow!("Protocol mismatch! Host: {}, Guest: {}", dyxel_shared::PROTOCOL_HASH, guest_hash)); 
        }
        
        let tick_fn = module.find_function::<(), ()>("guest_tick").or_else(|_| module.find_function::<(), ()>("vello_tick")).or_else(|_| module.find_function::<(), ()>("_guest_tick")).map_err(|_| anyhow::anyhow!("Tick func not found"))?;
        let on_click_fn = module.find_function::<(u32,), ()>("on_node_click").or_else(|_| module.find_function::<(u32,), ()>("_on_node_click")).map_err(|_| anyhow::anyhow!("OnClick func not found"))?;
        
        let _ = main_fn.call(); 
        let memory = unsafe { &mut *self._rt.memory_mut() }; 
        let _ = process_commands(memory, bptr, &self.shared_state);
        
        #[cfg(not(target_arch = "wasm32"))]
        log::info!("PERF: WASM Engine Initialization took {:?}", wasm_init_start.elapsed());

        *self.tick_fn.lock().unwrap() = Some(unsafe { std::mem::transmute(tick_fn) });
        *self.on_click_fn.lock().unwrap() = Some(unsafe { std::mem::transmute(on_click_fn) });
        *self.shared_buffer_ptr.lock().unwrap() = Some(bptr);

        Ok(())
    }
}
