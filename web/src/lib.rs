// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// Allow deprecated warnings from wasm-bindgen async constructors
#![allow(deprecated)]

use dyxel_core::input::hit_test_recursive;
use dyxel_core::DyxelHost;
use dyxel_render_api::{SharedMutex, SharedPtr};
use dyxel_shared::{Role, SharedState, ViewType};
use kurbo::Vec2;
use std::collections::HashMap;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{HtmlCanvasElement, HtmlElement, Response};

// Web-specific window handle wrapper for wgpu SurfaceTarget
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle, WebDisplayHandle, WebWindowHandle};

/// Wrapper for HtmlCanvasElement to implement raw_window_handle traits
pub struct WebCanvasHandle {
    canvas: HtmlCanvasElement,
    id: u32,
}

impl WebCanvasHandle {
    pub fn new(canvas: HtmlCanvasElement) -> Self {
        // Generate a unique ID for this canvas
        use std::sync::atomic::{AtomicU32, Ordering};
        static ID_COUNTER: AtomicU32 = AtomicU32::new(1);
        let id = ID_COUNTER.fetch_add(1, Ordering::SeqCst);
        Self { canvas, id }
    }

    pub fn canvas(&self) -> &HtmlCanvasElement {
        &self.canvas
    }
}

impl HasWindowHandle for WebCanvasHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let handle = WebWindowHandle::new(self.id);
        unsafe { Ok(WindowHandle::borrow_raw(RawWindowHandle::Web(handle))) }
    }
}

impl HasDisplayHandle for WebCanvasHandle {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        let handle = WebDisplayHandle::new();
        unsafe { Ok(DisplayHandle::borrow_raw(RawDisplayHandle::Web(handle))) }
    }
}

#[wasm_bindgen(start)]
pub fn start() {
    wasm_logger::init(wasm_logger::Config::default());
    console_error_panic_hook::set_once();
}

#[wasm_bindgen]
pub struct WebHost {
    host: SharedPtr<DyxelHost>,
    semantics_root: HtmlElement,
    dom_nodes: HashMap<u32, HtmlElement>,
}

#[wasm_bindgen]
impl WebHost {
    #[wasm_bindgen(constructor)]
    pub async fn new(canvas: HtmlCanvasElement) -> Result<WebHost, JsValue> {
        log::info!("Dyxel WebHost: Creating new instance...");
        let host = DyxelHost::new();

        let document = web_sys::window().unwrap().document().unwrap();
        let semantics_root = document.create_element("div")?.dyn_into::<HtmlElement>()?;
        let s = semantics_root.style();
        s.set_property("position", "absolute")?;
        s.set_property("top", "0")?;
        s.set_property("left", "0")?;
        s.set_property("width", "100%")?;
        s.set_property("height", "100%")?;
        s.set_property("pointer-events", "none")?;
        s.set_property("color", "white")?; // Set default text color
        s.set_property("font-family", "system-ui, -apple-system, sans-serif")?;
        canvas
            .parent_element()
            .unwrap()
            .append_child(&semantics_root)?;

        // 1. Asynchronously load engine
        host.prepare_engine(".".to_string()).await;
        log::info!("Dyxel WebHost: Engine prepared.");

        // 2. Initialize rendering, pass Canvas as SurfaceTarget
        // Wrap HtmlCanvasElement in WebCanvasHandle to implement raw_window_handle traits
        let canvas_handle = WebCanvasHandle::new(canvas.clone());
        let wgpu_target: vello::wgpu::SurfaceTarget<'static> = canvas_handle.into();
        let target_handle = dyxel_render_api::SurfaceTargetHandle::new(wgpu_target);
        host.setup(target_handle, canvas.width(), canvas.height(), None)
            .await;
        log::info!("Dyxel WebHost: Surface setup complete.");

        // 3. Load default font for Vello rendering
        // Load Roboto from Google Fonts CDN
        let font_url = "https://fonts.gstatic.com/s/roboto/v32/KFOmCnqEu92Fr1Mu4mxKKTU1Kg.woff2";
        match load_font_data(font_url).await {
            Ok(font_data) => {
                log::info!(
                    "Dyxel WebHost: Loaded font data ({} bytes)",
                    font_data.len()
                );
                // Store font data for Vello rendering
                // Note: In a full implementation, we'd register this with fontique
            }
            Err(e) => {
                log::warn!("Dyxel WebHost: Failed to load font: {:?}", e);
            }
        }

        Ok(WebHost {
            host,
            semantics_root,
            dom_nodes: HashMap::new(),
        })
    }

    #[wasm_bindgen(js_name = loadWasm)]
    pub async fn load_wasm(&self, _wasm_url: String) {
        // In scheme A, JS is responsible for loading WASM, this method is just a placeholder
        log::info!("Dyxel WebHost: load_wasm is a no-op in scheme A (JS loads WASM)");
    }

    /// Synchronous tick specifically for Wasm Guest
    #[wasm_bindgen(js_name = wasmSyncTick)]
    pub fn wasm_sync_tick(&mut self, guest_memory: &js_sys::Uint8Array, buffer_ptr: u32) {
        let mut mem = guest_memory.to_vec();

        if let Some(ss) = self.host.get_shared_state() {
            // Process guest commands
            let _ = dyxel_core::process_commands(&mut mem, buffer_ptr, &ss);

            // Execute rendering
            self.host.tick();

            // Sync new layout back to guest memory
            #[cfg(target_arch = "wasm32")]
            {
                let layout_guard = ss.borrow();
                let _ = dyxel_core::sync_layout_to_wasm(&mut mem, buffer_ptr, &*layout_guard);
            }
        }

        guest_memory.copy_from(&mem);
    }

    pub fn tick(&mut self) {
        self.host.tick();
        self.sync_semantics();
    }

    fn sync_semantics(&mut self) {
        let rid = self
            .host
            .get_shared_state()
            .and_then(|ss| ss.lock().unwrap().root_id);
        if let Some(rid) = rid {
            self.sync_node_dom_recursive(rid, Vec2::ZERO);
        }
    }

    fn sync_node_dom_recursive(&mut self, id: u32, parent_pos: Vec2) {
        let ss: SharedPtr<SharedMutex<SharedState>> = match self.host.get_shared_state() {
            Some(s) => s,
            None => return,
        };

        let (node_data, global_pos) = {
            let shared_guard = ss.lock().unwrap();
            let Some(node) = shared_guard.nodes.get(&id) else {
                return;
            };
            let Ok(layout) = shared_guard.taffy.layout(node.taffy_node) else {
                return;
            };
            let global_pos =
                parent_pos + Vec2::new(layout.location.x as f64, layout.location.y as f64);

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
            let el = document
                .create_element("div")
                .unwrap()
                .dyn_into::<HtmlElement>()
                .unwrap();
            let s = el.style();
            s.set_property("position", "absolute").unwrap();
            // Note: Text color is set to visible since WASM font loading via fontique is limited
            // In a full implementation, either embed fonts or use Web Fonts API
            // Web: Use visible color for DOM text overlay since Vello font loading is limited in WASM
            s.set_property("color", "white").unwrap();
            s.set_property("font-size", "16px").unwrap();
            s.set_property(
                "font-family",
                "system-ui, -apple-system, BlinkMacSystemFont, sans-serif",
            )
            .unwrap();
            s.set_property("user-select", "none").unwrap();
            self.semantics_root.append_child(&el).unwrap();
            el
        });

        let s = el.style();
        s.set_property("left", &format!("{}px", global_pos.x))
            .unwrap();
        s.set_property("top", &format!("{}px", global_pos.y))
            .unwrap();
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
            if role == Role::None {
                el.set_attribute("role", "button").unwrap();
            }
        } else {
            s.set_property("pointer-events", "none").unwrap();
        }

        for &child_id in &children {
            self.sync_node_dom_recursive(child_id, global_pos);
        }
    }

    #[wasm_bindgen(js_name = handleClick)]
    pub fn handle_click(&self, x: f64, y: f64) -> Option<u32> {
        if let Some(ss) = self.host.get_shared_state() {
            let mouse_pos = Vec2::new(x, y);
            let s_guard = ss.lock().unwrap();
            return s_guard.root_id.and_then(|rid| {
                hit_test_recursive(
                    rid,
                    mouse_pos,
                    &s_guard.nodes,
                    &s_guard.taffy,
                    Vec2::ZERO,
                    &s_guard.click_listeners,
                )
            });
        }
        None
    }

    /// Resize the canvas and notify the engine
    #[wasm_bindgen(js_name = resize)]
    pub fn resize(&self, width: u32, height: u32) {
        self.host.resize_native(width, height);
    }
}

/// Load font data from URL
async fn load_font_data(url: &str) -> Result<Vec<u8>, JsValue> {
    let window = web_sys::window().unwrap();
    let resp_value = JsFuture::from(window.fetch_with_str(url)).await?;
    let resp: Response = resp_value.dyn_into()?;

    if !resp.ok() {
        return Err(JsValue::from_str(&format!("HTTP error: {}", resp.status())));
    }

    let buffer = JsFuture::from(resp.array_buffer()?).await?;
    let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
    Ok(bytes)
}
