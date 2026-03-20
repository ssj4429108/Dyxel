use std::sync::{Arc, Mutex};
use vello::{Renderer, RendererOptions, util::RenderContext};
use crate::state::SharedState;

pub struct EngineState { 
    pub context: RenderContext, 
    pub renderer: Renderer, 
    pub shared_state: Arc<Mutex<SharedState>>,
    #[cfg(feature = "wasm3-support")] pub tick_fn: wasm3::Function<'static, (), ()>,
    #[cfg(feature = "wasm3-support")] pub on_click_fn: wasm3::Function<'static, (u32,), ()>,
    #[cfg(feature = "wasm3-support")] pub _rt: wasm3::Runtime,
    #[cfg(feature = "wasm3-support")] pub shared_buffer_ptr: u32,
    pub blit_bind_group_layout: vello::wgpu::BindGroupLayout, 
    pub sampler: vello::wgpu::Sampler, 
    pub blit_shader: vello::wgpu::ShaderModule,
}

unsafe impl Send for EngineState {}
unsafe impl Sync for EngineState {}

pub async fn setup_engine(_ddir: String, _es: Arc<Mutex<Option<EngineState>>>) -> anyhow::Result<EngineState> {
    let mut context = RenderContext::new(); 
    let dev_id = context.device(None).await.ok_or_else(|| anyhow::anyhow!("No device found"))?;
    let dev = &context.devices[dev_id].device;
    let blit_shader = dev.create_shader_module(vello::wgpu::ShaderModuleDescriptor { 
        label: Some("Blit Shader"), 
        source: vello::wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into()) 
    });
    let blit_bl = dev.create_bind_group_layout(&vello::wgpu::BindGroupLayoutDescriptor { 
        label: None, 
        entries: &[
            vello::wgpu::BindGroupLayoutEntry { 
                binding: 0, 
                visibility: vello::wgpu::ShaderStages::FRAGMENT, 
                ty: vello::wgpu::BindingType::Texture { 
                    sample_type: vello::wgpu::TextureSampleType::Float { filterable: true }, 
                    view_dimension: vello::wgpu::TextureViewDimension::D2, 
                    multisampled: false 
                }, 
                count: None 
            }, 
            vello::wgpu::BindGroupLayoutEntry { 
                binding: 1, 
                visibility: vello::wgpu::ShaderStages::FRAGMENT, 
                ty: vello::wgpu::BindingType::Sampler(vello::wgpu::SamplerBindingType::Filtering), 
                count: None 
            }
        ] 
    });
    let sampler = dev.create_sampler(&vello::wgpu::SamplerDescriptor { 
        mag_filter: vello::wgpu::FilterMode::Linear, 
        min_filter: vello::wgpu::FilterMode::Linear, 
        ..Default::default() 
    });
    let renderer = Renderer::new(dev, RendererOptions { 
        antialiasing_support: vello::AaSupport::all(), 
        pipeline_cache: None, 
        num_init_threads: None, 
        use_cpu: false 
    }).map_err(|e| anyhow::anyhow!("Failed to create renderer: {}", e))?;
    
    let state = Arc::new(Mutex::new(SharedState::new()));
    
    #[cfg(feature = "wasm3-support")] {
        use crate::runtime::process_commands;
        let wasm_path = format!("{}/guest.wasm", _ddir); 
        let wasm = std::fs::read(&wasm_path).or_else(|_| std::fs::read("guest.wasm")).map_err(|e| anyhow::anyhow!("Failed to read WASM: {}", e))?;
        let env = wasm3::Environment::new().map_err(|e| anyhow::anyhow!("Environment failed: {}", e))?; 
        let rt = env.create_runtime(1024 * 2048).map_err(|e| anyhow::anyhow!("Runtime failed: {}", e))?;
        let mut module = rt.load_module(env.parse_module(wasm).map_err(|e| anyhow::anyhow!("Parse failed: {}", e))?).map_err(|e| anyhow::anyhow!("Load failed: {}", e))?;
        let bptr = module.find_function::<(), u32>("vello_get_shared_buffer_ptr").map_err(|e| anyhow::anyhow!("Func not found: {}", e))?.call().map_err(|e| anyhow::anyhow!("Call failed: {}", e))?;
        
        let s_inner = state.clone();
        let _ = module.link_closure("env", "ui_force_layout", move |ctx, ()| { 
            let mem = unsafe { &mut *ctx.memory_mut() }; 
            let _ = process_commands(mem, bptr, &s_inner); 
            Ok(()) 
        });
        
        let main_fn = module.find_function::<(), ()>("main").or_else(|_| module.find_function::<(), ()>("_main")).map_err(|_| anyhow::anyhow!("Main not found"))?;
        let get_hash_fn = module.find_function::<(), u64>("vello_get_protocol_hash").map_err(|_| anyhow::anyhow!("vello_get_protocol_hash not found"))?;
        let guest_hash = get_hash_fn.call().map_err(|_| anyhow::anyhow!("Failed to call get_hash"))?;
        if guest_hash != shared::PROTOCOL_HASH { 
            return Err(anyhow::anyhow!("Protocol mismatch! Host: {}, Guest: {}", shared::PROTOCOL_HASH, guest_hash)); 
        }
        
        let tick_fn = module.find_function::<(), ()>("guest_tick").or_else(|_| module.find_function::<(), ()>("vello_tick")).or_else(|_| module.find_function::<(), ()>("_guest_tick")).map_err(|_| anyhow::anyhow!("Tick func not found"))?;
        let on_click_fn = module.find_function::<(u32,), ()>("on_node_click").or_else(|_| module.find_function::<(u32,), ()>("_on_node_click")).map_err(|_| anyhow::anyhow!("OnClick func not found"))?;
        
        let _ = main_fn.call(); 
        let memory = unsafe { &mut *rt.memory_mut() }; 
        let _ = process_commands(memory, bptr, &state);
        
        Ok(EngineState { 
            context, 
            renderer, 
            shared_state: state, 
            tick_fn: unsafe { std::mem::transmute(tick_fn) }, 
            on_click_fn: unsafe { std::mem::transmute(on_click_fn) }, 
            _rt: rt, 
            shared_buffer_ptr: bptr, 
            blit_bind_group_layout: blit_bl, 
            sampler, 
            blit_shader 
        })
    }
    #[cfg(not(feature = "wasm3-support"))] { 
        Ok(EngineState { context, renderer, shared_state: state, blit_bind_group_layout: blit_bl, sampler, blit_shader }) 
    }
}
