use std::sync::atomic::{AtomicU32, Ordering};
use vello_view::{View, BaseView, PositionType};

#[no_mangle]
pub extern "C" fn main() {
    // 1. 根容器 (ID 0)
    let _root = View::new()
        .width("100%")
        .height("100%")
        .color((10, 10, 40)); 
    
    // 2. 创建 100 个动态方块并挂载
    for _ in 1..101 {
        let child = View::new()
            .position(PositionType::Absolute)
            .width(30.0)
            .height(30.0);
        
        let _ = View { id: 0 }.child(child.id);
    }
}

static FRAME_COUNT: AtomicU32 = AtomicU32::new(0);

#[no_mangle]
pub extern "C" fn guest_tick() {
    let f = FRAME_COUNT.fetch_add(1, Ordering::SeqCst) as f32;
    
    for i in 1..101 {
        let idx = i as f32;
        // 使用正弦和余弦函数创建平滑的环形/随机运动
        // x 和 y 作为百分比 (0-100)
        let x = 50.0 + (f * 0.03 + idx * 0.5).cos() * 40.0; 
        let y = 50.0 + (f * 0.02 + idx * 0.3).sin() * 40.0; 
        
        let _ = View { id: i }
            // 修正参数顺序：(top, right, bottom, left)
            // 我们设置 top = y, left = x，并将 right/bottom 设为较大的值避免干扰布局
            .inset((y, 0.0, 0.0, x)) 
            // 颜色变换更平滑一些
            .color((
                (128.0 + (f * 0.02 + idx).cos() * 127.0) as u32,
                (128.0 + (f * 0.03 + idx * 0.5).sin() * 127.0) as u32,
                (128.0 + (idx * 2.0).cos() * 127.0) as u32
            ));
    }
    
    vello_view::vello_view_tick();
}
