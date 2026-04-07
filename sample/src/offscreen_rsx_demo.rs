// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Off-Screen Rendering (OSR) RSX 演示示例
//!
//! 本示例使用 #[app] 和 rsx! 宏实现
//! 展示 RSX DSL 对 offscreen、offscreenWithFilter、cacheHint 等属性的支持

use dyxel_app::prelude::*;
use dyxel_view::{ViewOffscreenExt, ViewFilterExt, CacheHint, filter_presets, BlendMode};

#[app]
pub fn OffscreenRSXDemo() -> impl BaseView {
    // 注册滤镜
    use_effect(|| {
        filter_presets::register_presets();
        dyxel_view::log("[OSR-RSX] 滤镜预设已注册: Light(2px), Medium(5px), Heavy(10px) blur");
    });

    // 透明度状态（静态值，实际项目中可以动态更新）
    let alpha = use_state(|| 0.5f32);

    rsx! {
        View {
            width: "100%",
            height: "100%",
            color: (254u32, 254, 254, 255), // #fefefe background
            flexDirection: FlexDirection::Column,
            alignItems: AlignItems::Center,
            justifyContent: JustifyContent::FlexStart,
            padding: (20.0, 20.0, 20.0, 20.0),

            // 标题
            Text {
                value: "Off-Screen Rendering (RSX)",
                fontSize: 20.0,
                textColor: (255u8, 255, 255, 255),
            }
            Text {
                value: "Using #[app] and rsx! macros with OSR attributes",
                fontSize: 12.0,
                textColor: (180u8, 180, 180, 255),
            }

            View { width: "100%", height: 20.0 }

            // 示例 1：半透明动画（使用 RSX offscreen 属性）
            Text {
                value: "1. Alpha Blending (Animated)",
                fontSize: 14.0,
                textColor: (100u8, 200, 255, 255),
            }

            View {
                width: "100%",
                height: 150.0,
                color: (230u32, 240, 250, 255), // light blue container
                flexDirection: FlexDirection::Column,
                justifyContent: JustifyContent::Center,
                alignItems: AlignItems::Center,

                // 条纹背景
                View { width: "100%", height: 18.0, color: (200u32, 210, 220, 255) }
                View { width: "100%", height: 18.0, color: (180u32, 190, 200, 255) }
                View { width: "100%", height: 18.0, color: (200u32, 210, 220, 255) }
                View { width: "100%", height: 18.0, color: (180u32, 190, 200, 255) }
                View { width: "100%", height: 18.0, color: (200u32, 210, 220, 255) }
                View { width: "100%", height: 18.0, color: (180u32, 190, 200, 255) }
                View { width: "100%", height: 18.0, color: (200u32, 210, 220, 255) }

                // 使用 RSX offscreen 属性！
                View {
                    width: 200.0,
                    height: 80.0,
                    color: (255u32, 100, 100, 255),
                    flexDirection: FlexDirection::Column,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,
                    // RSX 支持：offscreen 属性
                    offscreen: dyxel_view::OffscreenConfig::with_alpha(alpha.get()),

                    Text { value: "Fading Overlay", fontSize: 14.0, textColor: (255u8, 255, 255, 255) }
                    Text { value: "alpha: {alpha}", fontSize: 10.0, textColor: (255u8, 255, 255, 200) }
                }
            }

            View { width: "100%", height: 20.0 }

            // 示例 2：光栅缓存（使用 RSX cacheHint 属性）
            Text {
                value: "2. Raster Cache Hint",
                fontSize: 14.0,
                textColor: (100u8, 200, 255, 255),
            }

            View {
                width: "100%",
                height: 120.0,
                color: (240u32, 250, 240, 255), // light green container
                flexDirection: FlexDirection::Row,
                justifyContent: JustifyContent::SpaceEvenly,
                alignItems: AlignItems::Center,

                // 禁用缓存（使用 RSX cacheHint 属性）
                View {
                    width: 100.0,
                    height: 80.0,
                    color: (200u32, 80, 80, 255),
                    flexDirection: FlexDirection::Column,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,
                    offscreen: dyxel_view::OffscreenConfig::with_alpha(0.9),
                    // RSX 支持：cacheHint 属性
                    cacheHint: CacheHint::Disable,

                    Text { value: "No Cache", fontSize: 11.0, textColor: (255u8, 255, 255, 255) }
                    Text { value: "(Dynamic)", fontSize: 9.0, textColor: (200u8, 200, 200, 255) }
                }

                // 启用缓存（使用 RSX cacheHint 属性）
                View {
                    width: 100.0,
                    height: 80.0,
                    color: (80u32, 200, 80, 255),
                    flexDirection: FlexDirection::Column,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,
                    offscreen: dyxel_view::OffscreenConfig::with_alpha(0.9),
                    // RSX 支持：cacheHint 属性
                    cacheHint: CacheHint::Enable,

                    Text { value: "Cached", fontSize: 11.0, textColor: (255u8, 255, 255, 255) }
                    Text { value: "(Static)", fontSize: 9.0, textColor: (200u8, 200, 200, 255) }
                }
            }

            View { width: "100%", height: 20.0 }

            // 示例 3：模糊滤镜（使用 RSX offscreenWithFilter 属性）
            Text {
                value: "3. Blur Filter Effects",
                fontSize: 14.0,
                textColor: (100u8, 200, 255, 255),
            }

            View {
                width: "100%",
                height: 120.0,
                color: (245u32, 245, 250, 255), // light gray container
                flexDirection: FlexDirection::Row,
                justifyContent: JustifyContent::SpaceEvenly,
                alignItems: AlignItems::Center,

                // 原始（无模糊）
                View {
                    width: 60.0,
                    height: 80.0,
                    color: (100u32, 200, 100, 255),
                    flexDirection: FlexDirection::Column,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,
                    offscreen: dyxel_view::OffscreenConfig::with_alpha(0.95),

                    Text { value: "Original", fontSize: 9.0, textColor: (255u8, 255, 255, 255) }
                }

                // 轻微模糊（使用 RSX offscreenWithFilter 属性）
                View {
                    width: 60.0,
                    height: 80.0,
                    color: (100u32, 200, 100, 255),
                    flexDirection: FlexDirection::Column,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,
                    // RSX 支持：offscreenWithFilter 属性（元组形式）
                    offscreenWithFilter: (0.95, filter_presets::BLUR_LIGHT, BlendMode::Normal),

                    Text { value: "Light", fontSize: 9.0, textColor: (255u8, 255, 255, 255) }
                    Text { value: "2px", fontSize: 7.0, textColor: (200u8, 200, 200, 255) }
                }

                // 中等模糊
                View {
                    width: 60.0,
                    height: 80.0,
                    color: (100u32, 200, 100, 255),
                    flexDirection: FlexDirection::Column,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,
                    offscreenWithFilter: (0.95, filter_presets::BLUR_MEDIUM, BlendMode::Normal),

                    Text { value: "Medium", fontSize: 9.0, textColor: (255u8, 255, 255, 255) }
                    Text { value: "5px", fontSize: 7.0, textColor: (200u8, 200, 200, 255) }
                }

                // 强烈模糊
                View {
                    width: 60.0,
                    height: 80.0,
                    color: (100u32, 200, 100, 255),
                    flexDirection: FlexDirection::Column,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,
                    offscreenWithFilter: (0.95, filter_presets::BLUR_HEAVY, BlendMode::Normal),

                    Text { value: "Heavy", fontSize: 9.0, textColor: (255u8, 255, 255, 255) }
                    Text { value: "10px", fontSize: 7.0, textColor: (200u8, 200, 200, 255) }
                }
            }

            View { width: "100%", height: 20.0 }

            // 示例 4：阴影效果
            Text {
                value: "4. Drop Shadow Effects",
                fontSize: 14.0,
                textColor: (100u8, 200, 255, 255),
            }

            View {
                width: "100%",
                height: 80.0,
                color: (245u32, 245, 250, 255), // light gray container
                flexDirection: FlexDirection::Row,
                justifyContent: JustifyContent::SpaceEvenly,
                alignItems: AlignItems::Center,

                // 柔和阴影
                View {
                    width: 80.0,
                    height: 50.0,
                    color: (150u32, 150, 255, 255),
                    flexDirection: FlexDirection::Column,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,
                    offscreenWithFilter: (1.0, filter_presets::SHADOW_SOFT, BlendMode::Normal),

                    Text { value: "Soft Shadow", fontSize: 9.0, textColor: (255u8, 255, 255, 255) }
                }

                // 彩色阴影
                View {
                    width: 80.0,
                    height: 50.0,
                    color: (255u32, 150, 150, 255),
                    flexDirection: FlexDirection::Column,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,
                    offscreenWithFilter: (1.0, filter_presets::SHADOW_COLORED, BlendMode::Normal),

                    Text { value: "Colored", fontSize: 9.0, textColor: (255u8, 255, 255, 255) }
                }
            }

            View { width: "100%", height: 20.0 }

            // 说明
            Text {
                value: "RSX OSR Attributes: offscreen, offscreenWithFilter, cacheHint",
                fontSize: 11.0,
                textColor: (255u8, 200, 100, 255),
            }
        }
    }
}
