// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! perf_mixed_heavy - 混合压力基准
//!
//! 目标：模拟更接近真实复杂界面的综合压力，同时压 logic 和 render
//! 特征：中等规模节点，持续状态变化，保留一定数量 shadow/text
//!
//! 预期表现：UI FPS 和 Raster FPS 都会受压，Jank/Dropped 更接近真实界面行为

use dyxel_app::prelude::*;
use dyxel_view::{BaseView, FlexDirection, FlexWrap, JustifyContent, View, Text, set_text};
use std::cell::Cell;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::task::{Context, Poll};

// === 可调参数 ===
/// 总节点数
const TOTAL_NODES: u32 = 500;
/// 文本节点数
const TEXT_NODE_COUNT: u32 = 80;
/// blur / frosted glass 节点数
const BLUR_NODE_COUNT: u32 = 60;
/// shadow 节点数
const SHADOW_NODE_COUNT: u32 = 90;
/// opacity 节点数
const OPACITY_NODE_COUNT: u32 = 60;
/// 每帧更新节点数（约 30%）
const UPDATES_PER_FRAME: u32 = 180;
/// 动画速度系数
const ANIMATION_SPEED: f32 = 0.04;

// === 运行时状态 ===
static ROOT_ID: AtomicU32 = AtomicU32::new(0);

// 简单 LCG 伪随机数生成器
static mut RNG_STATE: u32 = 54321;

fn lcg_random() -> u32 {
    unsafe {
        RNG_STATE = RNG_STATE.wrapping_mul(1103515245).wrapping_add(12345);
        RNG_STATE
    }
}

fn random_u32(max: u32) -> u32 {
    if max == 0 {
        return 0;
    }
    lcg_random() % max
}

fn hsv_to_rgb(h: u32, s: f32, v: f32) -> (u32, u32, u32) {
    let h = h % 360;
    let c = v * s;
    let x = c * (1.0 - ((h as f32 / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h / 60 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (
        ((r + m) * 255.0) as u32,
        ((g + m) * 255.0) as u32,
        ((b + m) * 255.0) as u32,
    )
}

// 自定义 Future：每帧执行一次混合更新逻辑
struct MixedDriver {
    frame: Cell<u32>,
    root_id: Cell<u32>,
}

impl Future for MixedDriver {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let f = this.frame.get() + 1;
        this.frame.set(f);

        if f % 60 == 0 {
            dyxel_view::log(&format!("[MixedHeavy] frame={}", f));
        }

        // Run indefinitely for long-duration governor testing
        if f >= 100000 {
            dyxel_view::log("[MixedHeavy] reached frame limit, stopping driver");
            return Poll::Ready(());
        }

        let root_id = this.root_id.get();
        for _ in 0..UPDATES_PER_FRAME {
            let node_idx = random_u32(TOTAL_NODES);
            let node_id = root_id + 1 + node_idx;

            if node_idx < TEXT_NODE_COUNT {
                // 文本节点：修改内容和颜色
                let value = ((f as f32 * ANIMATION_SPEED + node_idx as f32) % 100.0) as u32;
                set_text(node_id, &format!("{}", value));
            } else if node_idx < TEXT_NODE_COUNT + BLUR_NODE_COUNT {
                // 模糊节点：动态 blur 半径 + 透明度 + 颜色
                let phase = (f as f32 * 0.04 + node_idx as f32 * 0.15) % std::f32::consts::TAU;
                let radius = 3.0 + 14.0 * (1.0 + phase.sin()) * 0.5; // 3~17px 波动
                let opacity = 0.85 + 0.14 * phase.cos(); // 0.71~0.99
                View { id: node_id }.blur(radius);
                View { id: node_id }.opacity(opacity);
                let hue = (f * 5 + node_idx * 9) % 360;
                let color = hsv_to_rgb(hue, 0.5, 0.88);
                View { id: node_id }.color((color.0, color.1, color.2, 180u32));
            } else if node_idx < TEXT_NODE_COUNT + BLUR_NODE_COUNT + SHADOW_NODE_COUNT {
                // Shadow 节点：修改颜色和阴影偏移
                let hue = (f * 3 + node_idx * 11) % 360;
                let color = hsv_to_rgb(hue, 0.6, 0.8);
                View { id: node_id }.color((color.0, color.1, color.2, 255u32));
                let dx = 2.0 + (f as f32 * 0.02).sin() * 2.0;
                let dy = 3.0 + (f as f32 * 0.025).cos() * 2.0;
                View { id: node_id }.shadow((dx, dy, 8.0, 0x60000000u32));
            } else if node_idx < TEXT_NODE_COUNT + BLUR_NODE_COUNT + SHADOW_NODE_COUNT + OPACITY_NODE_COUNT {
                // Opacity 节点：动态透明度 + 颜色变化
                let phase = (f as f32 * 0.03 + node_idx as f32 * 0.1) % std::f32::consts::TAU;
                let opacity = 0.3 + 0.5 * (1.0 + phase.sin()) * 0.5; // 0.3 ~ 0.8 波动
                View { id: node_id }.opacity(opacity);
                let hue = (f * 4 + node_idx * 13) % 360;
                let color = hsv_to_rgb(hue, 0.7, 0.9);
                View { id: node_id }.color((color.0, color.1, color.2, 255u32));
            } else {
                // 普通节点：颜色 + 尺寸变化
                let update_type = random_u32(2);
                if update_type == 0 {
                    let hue = (f * 5 + node_idx * 7) % 360;
                    let color = hsv_to_rgb(hue, 0.65, 0.85);
                    View { id: node_id }.color((color.0, color.1, color.2, 255u32));
                } else {
                    let w = 25.0 + random_u32(40) as f32;
                    let h = 20.0 + random_u32(30) as f32;
                    View { id: node_id }.width(w);
                    View { id: node_id }.height(h);
                }
            }
        }

        Poll::Pending
    }
}

impl Unpin for MixedDriver {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_heavy_has_blur_nodes() {
        let source = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/perf_mixed_heavy.rs"
        ));
        let blur_call = [".bl", "ur("].concat();
        let blur_const = ["BLUR_", "NODE_COUNT"].concat();
        assert!(source.contains(&blur_call), "expected .blur() calls in source");
        assert!(source.contains(&blur_const), "expected BLUR_NODE_COUNT constant in source");
    }

    #[test]
    fn mixed_driver_finishes_without_trapping_at_frame_limit() {
        let mut driver = Box::pin(MixedDriver {
            frame: Cell::new(99999),
            root_id: Cell::new(1),
        });
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(&waker);

        assert!(matches!(driver.as_mut().poll(&mut cx), Poll::Ready(())));
    }
}

#[app]
pub fn MixedHeavy() -> impl BaseView {
    let root = View::new()
        .width("100%")
        .height("100%")
        .color((12u32, 12, 20, 255))
        .flex_direction(FlexDirection::Row)
        .flex_wrap(FlexWrap::Wrap)
        .justify_content(JustifyContent::Center);

    ROOT_ID.store(root.id, Ordering::SeqCst);

    let cols = 20u32;
    let cell_w = 800.0 / cols as f32;
    let rows = (TOTAL_NODES + cols - 1) / cols;
    let cell_h = 600.0 / rows.max(1) as f32;

    for i in 0..TOTAL_NODES {
        let base_color = hsv_to_rgb((i * 360 / TOTAL_NODES) as u32, 0.65, 0.85);

        if i < TEXT_NODE_COUNT {
            // 文本卡片
            let card = View::new()
                .width(cell_w - 2.0)
                .height(cell_h - 2.0)
                .color((base_color.0, base_color.1, base_color.2, 255u32))
                .border_radius(4.0);

            let text = Text::new()
                .value(&format!("M{}", i))
                .font_size(10.0)
                .text_color((255u8, 255, 255, 230));

            View { id: card.id }.child(text.id);
            View { id: root.id }.child(card.id);
        } else if i < TEXT_NODE_COUNT + BLUR_NODE_COUNT {
            // 毛玻璃模糊节点 — 半透明底色 + blur 半径 + 内部子元素
            let blur_radius = 4.0 + (i % 4) as f32 * 5.0; // 4, 9, 14, 19px
            let blur_card = View::new()
                .width(cell_w - 3.0)
                .height(cell_h - 3.0)
                .color((base_color.0, base_color.1, base_color.2, 180u32))
                .border_radius(8.0)
                .blur(blur_radius)
                .opacity(0.99);
            // 内部装饰子元素（让毛玻璃效果可见）
            let inner = View::new()
                .width((cell_w - 16.0).max(4.0))
                .height((cell_h - 16.0).max(4.0))
                .color((255u32 - base_color.0, 255 - base_color.1, 255 - base_color.2, 60u32))
                .border_radius(4.0);
            View { id: blur_card.id }.child(inner.id);
            View { id: root.id }.child(blur_card.id);
        } else if i < TEXT_NODE_COUNT + BLUR_NODE_COUNT + SHADOW_NODE_COUNT {
            // 阴影节点
            let shadow_colors = [
                0x40000000u32,
                0x60000000u32,
                0x80404040u32,
            ];
            let shadow_color = shadow_colors[(i as usize) % shadow_colors.len()];
            let shadow_node = View::new()
                .width(cell_w - 4.0)
                .height(cell_h - 4.0)
                .color((base_color.0, base_color.1, base_color.2, 255u32))
                .border_radius(6.0)
                .shadow((2.0, 3.0, 8.0, shadow_color));
            View { id: root.id }.child(shadow_node.id);
        } else if i < TEXT_NODE_COUNT + BLUR_NODE_COUNT + SHADOW_NODE_COUNT + OPACITY_NODE_COUNT {
            // Opacity 节点：动态透明度 + 颜色
            let opacity_node = View::new()
                .width(cell_w - 3.0)
                .height(cell_h - 3.0)
                .color((base_color.0, base_color.1, base_color.2, 255u32))
                .border_radius(5.0)
                .opacity(0.55);
            View { id: root.id }.child(opacity_node.id);
        } else {
            // 普通节点
            let plain = View::new()
                .width(cell_w - 2.0)
                .height(cell_h - 2.0)
                .color((base_color.0, base_color.1, base_color.2, 255u32))
                .border_radius(4.0);
            View { id: root.id }.child(plain.id);
        }
    }

    // 注册到 executor，每帧被 poll
    let driver = MixedDriver {
        frame: Cell::new(0),
        root_id: Cell::new(root.id),
    };
    dyxel_view::spawn(Box::pin(driver));

    root
}
