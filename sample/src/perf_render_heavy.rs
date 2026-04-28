// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! perf_render_heavy - 纯渲染压力基准
//!
//! 目标：优先压 render / GPU / backend
//! 特征：大量常驻节点，较多阴影、模糊、圆角、裁剪，尽量少做状态更新
//!
//! 预期表现：Raster FPS 比 UI FPS 更早下降，GPU time 明显升高

use dyxel_app::prelude::*;
use dyxel_view::{BaseView, FlexDirection, FlexWrap, JustifyContent, View, Text};
use std::cell::Cell;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::task::{Context, Poll};

// === 可调参数 ===
/// 总节点数
const TOTAL_NODES: u32 = 500;
/// 文本节点数
const TEXT_NODE_COUNT: u32 = 60;
/// blur 节点数
const BLUR_NODE_COUNT: u32 = 40;
/// shadow 节点数
const SHADOW_NODE_COUNT: u32 = 120;
/// 每帧仅更新少量节点（保持视觉变化但逻辑压力极低）
const UPDATES_PER_FRAME: u32 = 10;

// === 运行时状态 ===
static ROOT_ID: AtomicU32 = AtomicU32::new(0);

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

// 自定义 Future：每帧执行一次极少量更新，保持场景"活着"
struct RenderDriver {
    frame: Cell<u32>,
    root_id: Cell<u32>,
}

impl Future for RenderDriver {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let f = this.frame.get() + 1;
        this.frame.set(f);

        if f == 1 {
            dyxel_view::log("[RenderHeavy] driver first poll");
        }
        if f % 30 == 0 {
            dyxel_view::log(&format!("[RenderHeavy] frame={}", f));
        }

        if f >= 300 {
            dyxel_view::log("[RenderHeavy] reached frame limit, stopping driver");
            return Poll::Ready(());
        }

        let root_id = this.root_id.get();
        let time = f as f32 * 0.02;
        for i in 0..UPDATES_PER_FRAME {
            let node_id = root_id + 1 + i;

            // 透明度正弦脉动
            let opacity = 0.7 + (time + i as f32 * 0.5).sin() * 0.25;
            View { id: node_id }.opacity(opacity.clamp(0.0, 1.0));

            // 每第 3 个节点额外做轻微颜色偏移
            if i % 3 == 0 {
                let hue = ((f * 2 + i * 17) % 360) as u32;
                let color = hsv_to_rgb(hue, 0.5, 0.9);
                View { id: node_id }.color((color.0, color.1, color.2, 255u32));
            }
        }

        Poll::Pending
    }
}

impl Unpin for RenderDriver {}

#[app]
pub fn RenderHeavy() -> impl BaseView {
    dyxel_view::log("[RenderHeavy] main() starting");

    let root = View::new()
        .width("100%")
        .height("100%")
        .color((30u32, 30, 50, 255))
        .flex_direction(FlexDirection::Row)
        .flex_wrap(FlexWrap::Wrap)
        .justify_content(JustifyContent::Center);

    ROOT_ID.store(root.id, Ordering::SeqCst);
    dyxel_view::log(&format!("[RenderHeavy] root.id={}", root.id));

    let cols = 20u32;
    let cell_w = 800.0 / cols as f32;
    let rows = (TOTAL_NODES + cols - 1) / cols;
    let cell_h = 600.0 / rows.max(1) as f32;

    for i in 0..TOTAL_NODES {
        let base_color = hsv_to_rgb((i * 360 / TOTAL_NODES) as u32, 0.6, 0.9);

        if i < TEXT_NODE_COUNT {
            // 文本节点：放在彩色背景小卡片内，增加文字渲染量
            let card = View::new()
                .width(cell_w - 2.0)
                .height(cell_h - 2.0)
                .color((base_color.0, base_color.1, base_color.2, 255u32))
                .border_radius(4.0);

            let text = Text::new()
                .value(&format!("R{}", i))
                .font_size(9.0)
                .text_color((255u8, 255, 255, 220));

            View { id: card.id }.child(text.id);
            View { id: root.id }.child(card.id);
        } else if i < TEXT_NODE_COUNT + BLUR_NODE_COUNT {
            // Blur 节点：毛玻璃效果，GPU 重载
            let blur_node = View::new()
                .width(cell_w - 2.0)
                .height(cell_h - 2.0)
                .color((255u32, 255, 255, 180))
                .border_radius(8.0)
                .blur(20.0)
                .opacity(0.99);
            View { id: root.id }.child(blur_node.id);
        } else if i < TEXT_NODE_COUNT + BLUR_NODE_COUNT + SHADOW_NODE_COUNT {
            // Shadow 节点：阴影渲染压力
            let shadow_colors = [
                0x40000000u32,
                0x60000000u32,
                0x80000000u32,
                0xA0404040u32,
                0x50404040u32,
            ];
            let shadow_color = shadow_colors[(i as usize) % shadow_colors.len()];
            let shadow_node = View::new()
                .width(cell_w - 4.0)
                .height(cell_h - 4.0)
                .color((base_color.0, base_color.1, base_color.2, 255u32))
                .border_radius(6.0)
                .shadow((3.0, 4.0, 10.0, shadow_color));
            View { id: root.id }.child(shadow_node.id);
        } else {
            // 普通节点：圆角 + 不透明度，保持一定视觉复杂度
            let plain = View::new()
                .width(cell_w - 2.0)
                .height(cell_h - 2.0)
                .color((base_color.0, base_color.1, base_color.2, 255u32))
                .border_radius(4.0)
                .opacity(0.85);
            View { id: root.id }.child(plain.id);
        }
    }

    dyxel_view::log("[RenderHeavy] all nodes created, spawning driver");

    // 注册到 executor，每帧被 poll
    let driver = RenderDriver {
        frame: Cell::new(0),
        root_id: Cell::new(root.id),
    };
    dyxel_view::spawn(Box::pin(driver));
    dyxel_view::log("[RenderHeavy] driver spawned, returning root");

    root
}
