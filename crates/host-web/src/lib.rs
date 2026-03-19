use wasm_bindgen::prelude::*;
use std::sync::Arc;
use std::collections::HashMap;
use host_core::{VelloHost, Role, ViewType, hit_test_recursive};
use web_sys::{HtmlCanvasElement, HtmlElement};
use kurbo::Vec2;

#[wasm_bindgen(start)]
pub fn start() { console_error_panic_hook::set_once(); }

#[wasm_bindgen]
pub struct WebHost {
    host: Arc<VelloHost>,
    semantics_root: HtmlElement,
    dom_nodes: HashMap<u32, HtmlElement>,
}

#[wasm_bindgen]
impl WebHost {
    pub async fn create(canvas: HtmlCanvasElement, font_url: String) -> Result<WebHost, JsValue> {
        let host = VelloHost::new();
        
        let document = web_sys::window().unwrap().document().unwrap();
        let semantics_root = document.create_element("div")?.dyn_into::<HtmlElement>()?;
        let s = semantics_root.style();
        s.set_property("position", "absolute")?;
        s.set_property("top", "0")?; s.set_property("left", "0")?;
        s.set_property("width", "100%")?; s.set_property("height", "100%")?;
        s.set_property("pointer-events", "none")?;
        canvas.parent_element().unwrap().append_child(&semantics_root)?;

        // 初始化渲染，传入 Canvas 作为 SurfaceTarget
        host.setup(
            vello::wgpu::SurfaceTarget::Canvas(canvas.clone()),
            ".".to_string(), // data_dir
            canvas.width(),
            canvas.height(),
            None
        ).await;

        let font_data = load_font(font_url).await?;
        // 设置字体
        if let Some(s) = &mut *host.get_state_mut() {
            s.shared_state.lock().unwrap().set_font_data(font_data);
        }

        Ok(WebHost {
            host,
            semantics_root,
            dom_nodes: HashMap::new(),
        })
    }

    pub fn render(&mut self) {
        self.host.tick();
        // 如果是 Web 原生模式（无 Wasm Guest），仍可同步语义层
        self.sync_semantics();
    }

    /// 专门用于 Wasm Guest 的同步 Tick
    pub fn wasm_sync_tick(&mut self, guest_memory: &js_sys::Uint8Array, buffer_ptr: u32) {
        let mut mem = guest_memory.to_vec();
        
        // 1. 先提取 shared_state 句柄，立即释放 host 锁
        let ss_arc = {
            let guard = self.host.get_state();
            guard.as_ref().map(|e| e.shared_state.clone())
        };

        if let Some(ss) = ss_arc {
            // 2. 处理 Guest 指令 (内部会锁 ss)
            let _ = host_core::process_commands(&mut mem, buffer_ptr, &ss);
            
            // 3. 宿主渲染及布局计算 (内部会锁 host.engine 和 ss)
            self.host.tick();
            
            // 4. 将新布局同步回 Guest 内存 (此时 tick 已完成，安全锁 ss)
            let _ = host_core::sync_layout_to_wasm(&mut mem, buffer_ptr, &ss.lock().unwrap());
        }
        
        // 5. 写回 Guest 内存
        guest_memory.copy_from(&mem);
    }

    fn sync_semantics(&mut self) {
        let rid = {
            let s_opt = &*self.host.get_state();
            if let Some(s) = s_opt {
                s.shared_state.lock().unwrap().root_id
            } else {
                None
            }
        };

        if let Some(rid) = rid {
            self.sync_node_dom_recursive(rid, Vec2::ZERO);
        }
    }

    fn sync_node_dom_recursive(&mut self, id: u32, parent_pos: Vec2) {
        let (node_data, global_pos) = {
            let s_opt = &*self.host.get_state();
            let s = s_opt.as_ref().unwrap();
            let shared_guard = s.shared_state.lock().unwrap();
            let node = shared_guard.nodes.get(&id).unwrap();
            let layout = shared_guard.taffy.layout(node.taffy_node).unwrap();
            let global_pos = parent_pos + Vec2::new(layout.location.x as f64, layout.location.y as f64);
            
            (
                (
                    node.view_type,
                    node.text.clone(),
                    node.label.clone(),
                    node.has_click,
                    node.role,
                    layout.size.width,
                    layout.size.height,
                    node.children.clone(),
                ),
                global_pos,
            )
        };

        let (view_type, text, label, has_click, role, width, height, children) = node_data;

        let el = self.dom_nodes.entry(id).or_insert_with(|| {
            let document = web_sys::window().unwrap().document().unwrap();
            let el = document.create_element("div").unwrap().dyn_into::<HtmlElement>().unwrap();
            let s = el.style();
            s.set_property("position", "absolute").unwrap();
            s.set_property("color", "transparent").unwrap();
            s.set_property("user-select", "none").unwrap();
            self.semantics_root.append_child(&el).unwrap();
            el
        });

        let s = el.style();
        s.set_property("left", &format!("{}px", global_pos.x)).unwrap();
        s.set_property("top", &format!("{}px", global_pos.y)).unwrap();
        s.set_property("width", &format!("{}px", width)).unwrap();
        s.set_property("height", &format!("{}px", height)).unwrap();

        if view_type == ViewType::Text {
            el.set_inner_text(&text);
        } else if !label.is_empty() {
            el.set_attribute("aria-label", &label).unwrap();
        }

        if has_click {
            s.set_property("pointer-events", "auto").unwrap();
            s.set_property("cursor", "pointer").unwrap();
            el.set_attribute("tabindex", "0").unwrap();
            if role == Role::None { el.set_attribute("role", "button").unwrap(); }
        } else {
            s.set_property("pointer-events", "none").unwrap();
        }

        for &child_id in &children {
            self.sync_node_dom_recursive(child_id, global_pos);
        }
    }

    pub fn handle_click(&self, x: f64, y: f64) -> Option<u32> {
        if let Some(s) = &*self.host.get_state() {
            let mouse_pos = Vec2::new(x, y);
            let s_guard = s.shared_state.lock().unwrap();
            return s_guard.root_id.and_then(|rid| {
                hit_test_recursive(rid, mouse_pos, &s_guard.nodes, &s_guard.taffy, Vec2::ZERO, &s_guard.click_listeners)
            });
        }
        None
    }

    pub fn apply_commands(&self, command_data: &[u8]) {
        if let Some(s) = &*self.host.get_state() {
            let _ = host_core::process_command_stream(&s.shared_state, command_data);
        }
    }

    pub fn force_layout(&self, width: u32, height: u32) {
        let e_guard = self.host.get_state();
        if let Some(e) = &*e_guard {
            let mut g = e.shared_state.lock().unwrap();
            if let Some(rid) = g.root_id {
                if let Some(rn) = g.nodes.get(&rid).map(|n| n.taffy_node) {
                    let _ = g.taffy.compute_layout(rn, taffy::prelude::Size {
                        width: taffy::prelude::AvailableSpace::Definite(width as f32),
                        height: taffy::prelude::AvailableSpace::Definite(height as f32),
                    });
                }
            }
        }
    }

    pub fn get_layout_buffer(&self) -> Vec<f32> {
        let mut results = Vec::new();
        if let Some(e) = &*self.host.get_state() {
            let g = e.shared_state.lock().unwrap();
            for id in 0..shared::MAX_NODES as u32 {
                if let Some(node) = g.nodes.get(&id) {
                    let layout = g.taffy.layout(node.taffy_node).unwrap();
                    results.push(layout.location.x);
                    results.push(layout.location.y);
                    results.push(layout.size.width);
                    results.push(layout.size.height);
                } else {
                    results.push(0.0); results.push(0.0); results.push(0.0); results.push(0.0);
                }
            }
        }
        results
    }
}

async fn load_font(url: String) -> Result<Vec<u8>, JsValue> {
    let window = web_sys::window().unwrap();
    let resp_value = wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(&url)).await?;
    let resp: web_sys::Response = resp_value.dyn_into()?;
    let array_buffer_value = wasm_bindgen_futures::JsFuture::from(resp.array_buffer()?).await?;
    let array_buffer: js_sys::ArrayBuffer = array_buffer_value.dyn_into()?;
    Ok(js_sys::Uint8Array::new(&array_buffer).to_vec())
}
