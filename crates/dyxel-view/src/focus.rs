// crates/dyxel-view/src/focus.rs
use std::sync::atomic::{AtomicU32, Ordering};

static FOCUSED_ID: AtomicU32 = AtomicU32::new(0);

pub fn request_focus(id: u32) {
    let prev = FOCUSED_ID.swap(id, Ordering::SeqCst);
    if prev != 0 && prev != id {
        // TODO: 触发旧节点的 blur 回调 (在后续组件实现中补充)
        log::debug!("Focus preempted: {} -> {}", prev, id);
    }
}

pub fn get_focused_id() -> u32 {
    FOCUSED_ID.load(Ordering::SeqCst)
}

pub fn clear_focus() {
    FOCUSED_ID.store(0, Ordering::SeqCst);
}
