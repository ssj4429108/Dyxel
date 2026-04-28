// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! perf_logic_heavy - 纯逻辑压力基准
//!
//! 目标：优先压 logic / runtime_prepare() / layout / scene snapshot rebuild
//! 特征：大量节点，每帧高比例修改位置、尺寸、颜色、文本
//!
//! 预期表现：UI FPS 先掉，LogicTime / RuntimePrepare 更早变差

use dyxel_app::prelude::*;
use dyxel_view::{BaseView, FlexDirection, FlexWrap, JustifyContent, View, Text, set_text};
use std::cell::Cell;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::task::{Context, Poll};

// === 可调参数 ===
/// 总节点数
const TOTAL_NODES: u32 = 800;
/// 文本节点数
const TEXT_NODE_COUNT: u32 = 200;
/// 每帧更新节点数（约 50%）
const UPDATES_PER_FRAME: u32 = 400;
/// 动画速度系数
const ANIMATION_SPEED: f32 = 0.05;

// === 运行时状态 ===
static ROOT_ID: AtomicU32 = AtomicU32::new(0);

// 简单 LCG 伪随机数生成器
static mut RNG_STATE: u32 = 12345;

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

// 自定义 Future：每帧执行一次高比例节点更新
struct LogicDriver {
    frame: Cell<u32>,
    root_id: Cell<u32>,
}

impl Future for LogicDriver {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let f = this.frame.get() + 1;
        this.frame.set(f);

        if f % 60 == 0 {
            dyxel_view::log(&format!("[LogicHeavy] frame={}", f));
        }

        if f >= 300 {
            dyxel_view::log("[LogicHeavy] reached frame limit, stopping driver");
            return Poll::Ready(());
        }

        let root_id = this.root_id.get();
        for _ in 0..UPDATES_PER_FRAME {
            let node_idx = random_u32(TOTAL_NODES);
            let node_id = root_id + 1 + node_idx;

            if node_idx < TEXT_NODE_COUNT {
                // 文本节点：修改内容和颜色
                let value = ((f as f32 * ANIMATION_SPEED + node_idx as f32) % 1000.0) as u32;
                set_text(node_id, &format!("{}", value));
            } else {
                // 普通节点：颜色 + 尺寸变化
                let update_type = random_u32(3);
                if update_type == 0 {
                    let hue = (f * 7 + node_idx * 13) % 360;
                    let color = hsv_to_rgb(hue, 0.7, 0.9);
                    View { id: node_id }.color((color.0, color.1, color.2, 255u32));
                } else if update_type == 1 {
                    let w = 20.0 + random_u32(50) as f32;
                    View { id: node_id }.width(w);
                } else {
                    let h = 15.0 + random_u32(40) as f32;
                    View { id: node_id }.height(h);
                }
            }
        }

        Poll::Pending
    }
}

impl Unpin for LogicDriver {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logic_driver_finishes_without_trapping_at_frame_limit() {
        let mut driver = Box::pin(LogicDriver {
            frame: Cell::new(299),
            root_id: Cell::new(1),
        });
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(&waker);

        assert!(matches!(driver.as_mut().poll(&mut cx), Poll::Ready(())));
    }
}

#[app]
pub fn LogicHeavy() -> impl BaseView {
    let root = View::new()
        .width("100%")
        .height("100%")
        .color((8u32, 8, 12, 255))
        .flex_direction(FlexDirection::Row)
        .flex_wrap(FlexWrap::Wrap)
        .justify_content(JustifyContent::Center);

    ROOT_ID.store(root.id, Ordering::SeqCst);

    let cols = 25u32;
    let cell_w = 800.0 / cols as f32;
    let rows = (TOTAL_NODES + cols - 1) / cols;
    let cell_h = 600.0 / rows.max(1) as f32;

    for i in 0..TOTAL_NODES {
        let base_color = hsv_to_rgb((i * 360 / TOTAL_NODES) as u32, 0.6, 0.85);

        if i < TEXT_NODE_COUNT {
            // 文本节点
            let card = View::new()
                .width(cell_w - 2.0)
                .height(cell_h - 2.0)
                .color((base_color.0, base_color.1, base_color.2, 255u32))
                .border_radius(3.0);

            let text = Text::new()
                .value(&format!("L{}", i))
                .font_size(9.0)
                .text_color((255u8, 255, 255, 220));

            View { id: card.id }.child(text.id);
            View { id: root.id }.child(card.id);
        } else {
            // 普通节点
            let plain = View::new()
                .width(cell_w - 2.0)
                .height(cell_h - 2.0)
                .color((base_color.0, base_color.1, base_color.2, 255u32))
                .border_radius(3.0);
            View { id: root.id }.child(plain.id);
        }
    }

    let driver = LogicDriver {
        frame: Cell::new(0),
        root_id: Cell::new(root.id),
    };
    dyxel_view::spawn(Box::pin(driver));

    root
}
