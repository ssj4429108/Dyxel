// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Gesture Orchestration Test
//!
//! Tests gesture system including:
//! - Basic Gestures: Tap, DoubleTap, LongPress, Pan
//! - Gesture Arena: Competing gestures (Tap vs LongPress)
//! - RSX Gesture DSL
//!
//! ## RSX Gesture DSL
//!
//! ### Basic Gestures (已支持)
//! ```rust
//! View {
//!     onTap: move |_| { /* handle tap */ },
//!     onDoubleTap: move |_| { /* handle double tap */ },
//!     onLongPress: move |_| { /* handle long press */ },
//!     onPanUpdate: move |event| { /* handle pan */ },
//! }
//! ```
//!
//! ### Composite Gestures (RSX DSL 语法已支持，执行 WIP)
//! ```rust
//! View {
//!     gesture: SequenceGesture::new(vec![
//!         TapGesture::double_tap().into(),
//!         LongPressGesture::new().into(),
//!     ]),
//! }
//! 
//! View {
//!     gesture: ExclusiveGesture::new(vec![
//!         TapGesture::single_tap().into(),
//!         LongPressGesture::new().into(),
//!     ]),
//! }
//! 
//! View {
//!     gesture: ParallelGesture::new(vec![
//!         PanGesture::new().into(),
//!         PinchGesture::new().into(),
//!     ]),
//! }
//! ```

use dyxel_app::prelude::*;

#[app]
pub fn GestureOrchestration() -> impl BaseView {
    // 基础手势统计
    let tap_count = use_state(|| 0u32);
    let double_tap_count = use_state(|| 0u32);
    let long_press_count = use_state(|| 0u32);
    let pan_count = use_state(|| 0u32);
    
    // 竞技场统计
    let exclusive_tap_wins = use_state(|| 0u32);
    let exclusive_long_wins = use_state(|| 0u32);
    
    // 事件日志
    let event_log = use_state(|| "Ready...".to_string());
    
    // 可视化演示状态 (0=默认灰, 1=绿色Tap, 2=橙色LongPress)
    let demo_state = use_state(|| 0u32);
    let demo_text = use_state(|| "Touch me".to_string());
    
    // 动态属性演示
    let dynamic_size = use_state(|| 50.0f32);
    let dynamic_color = use_state(|| (100u32, 100, 200, 255));
    
    rsx! {
        View {
            width: "100%",
            height: "100%",
            color: (15u32, 15, 25, 255),
            flexDirection: FlexDirection::Column,
            alignItems: AlignItems::Center,
            padding: (10.0, 10.0, 10.0, 10.0),
            
            // 标题
            Text("Gesture Orchestration") {
                fontSize: 22.0,
                textColor: (0u8, 0, 0, 255),
                margin: (0.0, 0.0, 10.0, 0.0),
            }
            
            // 基础手势区域
            View {
                width: "95%",
                height: 200.0,
                color: (30u32, 30, 45, 255),
                borderRadius: 12.0,
                margin: (0.0, 0.0, 10.0, 0.0),
                flexDirection: FlexDirection::Column,
                padding: (10.0, 10.0, 10.0, 10.0),
                
                Text("Basic Gestures (RSX DSL)") {
                    fontSize: 14.0,
                    textColor: (0u8, 0, 0, 255),
                    margin: (0.0, 0.0, 10.0, 0.0),
                }
                
                // 手势按钮行
                View {
                    width: "100%",
                    flexDirection: FlexDirection::Row,
                    justifyContent: JustifyContent::SpaceAround,
                    
                    // Tap 按钮 - 使用 onTap DSL
                    View {
                        width: 70.0,
                        height: 70.0,
                        color: (60u32, 140, 240, 255),
                        borderRadius: 12.0,
                        justifyContent: JustifyContent::Center,
                        alignItems: AlignItems::Center,
                        onTap: {
                            let count = tap_count.clone();
                            let log = event_log.clone();
                            move |_| {
                                let c = count.get() + 1;
                                count.set(c);
                                log.set(format!("onTap triggered! (#{})", c));
                            }
                        },
                        
                        Text("Tap") {
                            fontSize: 12.0,
                            textColor: (255u8, 255, 255, 255),
                        }
                        
                        Text("{tap_count}") {
                            fontSize: 20.0,
                            fontWeight: 700,
                            textColor: (255u8, 255, 255, 255),
                            margin: (5.0, 0.0, 0.0, 0.0),
                        }
                    }
                    
                    // Double Tap 按钮 - 使用 onDoubleTap DSL
                    View {
                        width: 70.0,
                        height: 70.0,
                        color: (140u32, 60, 240, 255),
                        borderRadius: 12.0,
                        justifyContent: JustifyContent::Center,
                        alignItems: AlignItems::Center,
                        onDoubleTap: {
                            let count = double_tap_count.clone();
                            let log = event_log.clone();
                            move |_| {
                                let c = count.get() + 1;
                                count.set(c);
                                log.set(format!("onDoubleTap triggered! (#{})", c));
                            }
                        },
                        
                        Text("Double") {
                            fontSize: 12.0,
                            textColor: (255u8, 255, 255, 255),
                        }
                        
                        Text("{double_tap_count}") {
                            fontSize: 20.0,
                            fontWeight: 700,
                            textColor: (255u8, 255, 255, 255),
                            margin: (5.0, 0.0, 0.0, 0.0),
                        }
                    }
                    
                    // Long Press 按钮 - 使用 onLongPress DSL
                    View {
                        width: 70.0,
                        height: 70.0,
                        color: (240u32, 140, 60, 255),
                        borderRadius: 12.0,
                        justifyContent: JustifyContent::Center,
                        alignItems: AlignItems::Center,
                        onLongPress: {
                            let count = long_press_count.clone();
                            let log = event_log.clone();
                            move |_| {
                                let c = count.get() + 1;
                                count.set(c);
                                log.set(format!("onLongPress triggered! (#{})", c));
                            }
                        },
                        
                        Text("Long") {
                            fontSize: 12.0,
                            textColor: (255u8, 255, 255, 255),
                        }
                        
                        Text("{long_press_count}") {
                            fontSize: 20.0,
                            fontWeight: 700,
                            textColor: (255u8, 255, 255, 255),
                            margin: (5.0, 0.0, 0.0, 0.0),
                        }
                    }
                    
                    // Pan 按钮 - 使用 onPanUpdate DSL
                    View {
                        width: 70.0,
                        height: 70.0,
                        color: (60u32, 200, 120, 255),
                        borderRadius: 12.0,
                        justifyContent: JustifyContent::Center,
                        alignItems: AlignItems::Center,
                        onPanUpdate: {
                            let count = pan_count.clone();
                            let log = event_log.clone();
                            move |_| {
                                let c = count.get() + 1;
                                count.set(c);
                                if c % 10 == 0 {
                                    log.set(format!("onPanUpdate x{}", c));
                                }
                            }
                        },
                        
                        Text("Pan") {
                            fontSize: 12.0,
                            textColor: (255u8, 255, 255, 255),
                        }
                        
                        Text("{pan_count}") {
                            fontSize: 20.0,
                            fontWeight: 700,
                            textColor: (255u8, 255, 255, 255),
                            margin: (5.0, 0.0, 0.0, 0.0),
                        }
                    }
                }
            }
            
            // RSX Gesture DSL 说明和复合手势演示
            View {
                width: "95%",
                height: 150.0,
                color: (40u32, 40, 55, 255),
                borderRadius: 12.0,
                margin: (0.0, 0.0, 10.0, 0.0),
                flexDirection: FlexDirection::Column,
                justifyContent: JustifyContent::Center,
                alignItems: AlignItems::Center,
                padding: (10.0, 10.0, 10.0, 10.0),
                
                Text("RSX Gesture DSL") {
                    fontSize: 14.0,
                    textColor: (0u8, 0, 0, 255),
                }
                
                Text("Basic: onTap/onDoubleTap/onLongPress/onPanUpdate") {
                    fontSize: 11.0,
                    textColor: (0u8, 0, 0, 255),
                    margin: (5.0, 0.0, 2.0, 0.0),
                }
                
                Text("Composite: gesture: SequenceGesture([...])") {
                    fontSize: 11.0,
                    textColor: (0u8, 0, 0, 255),
                }
                
                // gesture DSL 配置示例（已完整实现）
                View {
                    width: "90%",
                    height: 30.0,
                    color: (60u32, 60, 80, 255),
                    borderRadius: 6.0,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,
                    margin: (8.0, 0.0, 0.0, 0.0),
                    // ExclusiveGesture: 多个手势竞争，只有一个获胜
                    gesture: {
                        use dyxel_view::{ExclusiveGesture, TapGesture, LongPressGesture};
                        ExclusiveGesture::new(vec![
                            TapGesture::single_tap().into(),
                            LongPressGesture::new().into(),
                        ])
                    },
                    onTap: {
                        let log = event_log.clone();
                        move |_| log.set("ExclusiveGesture: Tap won!".to_string())
                    },
                    onLongPress: {
                        let log = event_log.clone();
                        move |_| log.set("ExclusiveGesture: LongPress won!".to_string())
                    },
                    
                    Text("gesture: ExclusiveGesture (Tap vs LongPress)") {
                        fontSize: 10.0,
                        textColor: (200u8, 200, 255, 255),
                    }
                }
                
                Text("(Composite gesture fully implemented)") {
                    fontSize: 10.0,
                    textColor: (100u8, 200, 100, 255),
                    margin: (5.0, 0.0, 0.0, 0.0),
                }
            }
            
            // 手势竞技场 - 互斥手势
            View {
                width: "95%",
                height: 140.0,
                color: (55u32, 40, 40, 255),
                borderRadius: 12.0,
                margin: (0.0, 0.0, 10.0, 0.0),
                flexDirection: FlexDirection::Column,
                padding: (10.0, 10.0, 10.0, 10.0),
                
                Text("Gesture Arena (Tap vs LongPress)") {
                    fontSize: 14.0,
                    textColor: (0u8, 0, 0, 255),
                    margin: (0.0, 0.0, 5.0, 0.0),
                }
                
                // 统计
                View {
                    width: "100%",
                    flexDirection: FlexDirection::Row,
                    justifyContent: JustifyContent::SpaceAround,
                    margin: (0.0, 0.0, 10.0, 0.0),
                    
                    View {
                        flexDirection: FlexDirection::Column,
                        alignItems: AlignItems::Center,
                        
                        Text("Tap Wins") {
                            fontSize: 11.0,
                            textColor: (0u8, 0, 0, 255),
                        }
                        Text("{exclusive_tap_wins}") {
                            fontSize: 18.0,
                            fontWeight: 700,
                            textColor: (60u8, 140, 240, 255),
                        }
                    }
                    
                    View {
                        flexDirection: FlexDirection::Column,
                        alignItems: AlignItems::Center,
                        
                        Text("Long Wins") {
                            fontSize: 11.0,
                            textColor: (0u8, 0, 0, 255),
                        }
                        Text("{exclusive_long_wins}") {
                            fontSize: 18.0,
                            fontWeight: 700,
                            textColor: (240u8, 140, 60, 255),
                        }
                    }
                }
                
                // 竞争区域 - 同时注册两种手势（RSX DSL 支持多手势属性）
                View {
                    width: "90%",
                    height: 40.0,
                    color: (80u32, 80, 100, 255),
                    borderRadius: 8.0,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,
                    // 同时注册 onTap 和 onLongPress，系统会自动竞争
                    onTap: {
                        let tap_wins = exclusive_tap_wins.clone();
                        let long_wins = exclusive_long_wins.clone();
                        let log = event_log.clone();
                        move |_| {
                            let wins = tap_wins.get() + 1;
                            tap_wins.set(wins);
                            log.set(format!("Arena: Tap won! (Tap:{}, Long:{})", 
                                wins, long_wins.get()));
                        }
                    },
                    onLongPress: {
                        let tap_wins = exclusive_tap_wins.clone();
                        let long_wins = exclusive_long_wins.clone();
                        let log = event_log.clone();
                        move |_| {
                            let wins = long_wins.get() + 1;
                            long_wins.set(wins);
                            log.set(format!("Arena: LongPress won! (Tap:{}, Long:{})", 
                                tap_wins.get(), wins));
                        }
                    },
                    
                    Text("Tap quickly OR Hold for LongPress") {
                        fontSize: 12.0,
                        textColor: (255u8, 255, 255, 255),
                    }
                }
            }
            
            // 可视化演示 - 点击/长按颜色变化
            View {
                width: "95%",
                height: 100.0,
                color: (35u32, 35, 50, 255),
                borderRadius: 12.0,
                margin: (0.0, 0.0, 10.0, 0.0),
                flexDirection: FlexDirection::Column,
                justifyContent: JustifyContent::Center,
                alignItems: AlignItems::Center,
                
                Text("Visual Demo - Touch Feedback") {
                    fontSize: 14.0,
                    textColor: (0u8, 0, 0, 255),
                    margin: (0.0, 0.0, 5.0, 0.0),
                }
                
                // 颜色变化区域 (根据 demo_state 变化颜色)
                View {
                    width: 200.0,
                    height: 40.0,
                    color: {
                        let s = demo_state.get();
                        let c: (u32, u32, u32, u32) = if s == 1 { (60, 200, 120, 255) }      // 绿色 = Tap
                        else if s == 2 { (240, 100, 60, 255) } // 橙色 = LongPress
                        else { (100, 100, 120, 255) };          // 默认灰色
                        c
                    },
                    borderRadius: 8.0,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,
                    onTap: {
                        let state = demo_state.clone();
                        let text = demo_text.clone();
                        let log = event_log.clone();
                        move |_| {
                            state.set(1u32); // 绿色 = Tap
                            text.set("Tap!".to_string());
                            log.set("Visual: Tap detected!".to_string());
                        }
                    },
                    onLongPress: {
                        let state = demo_state.clone();
                        let text = demo_text.clone();
                        let log = event_log.clone();
                        move |_| {
                            state.set(2u32); // 橙色 = LongPress
                            text.set("LongPress!".to_string());
                            log.set("Visual: LongPress detected!".to_string());
                        }
                    },
                    
                    Text("{demo_text}") {
                        fontSize: 14.0,
                        fontWeight: 600,
                        textColor: (255u8, 255, 255, 255),
                    }
                }
                
                Text("(Tap=Green, LongPress=Orange)") {
                    fontSize: 10.0,
                    textColor: (0u8, 0, 0, 255),
                    margin: (5.0, 0.0, 0.0, 0.0),
                }
            }
            
            // 动态属性演示 - State 绑定
            View {
                width: "95%",
                height: 100.0,
                color: (45u32, 45, 60, 255),
                borderRadius: 12.0,
                margin: (0.0, 0.0, 10.0, 0.0),
                flexDirection: FlexDirection::Column,
                justifyContent: JustifyContent::Center,
                alignItems: AlignItems::Center,
                
                Text("Dynamic State Binding") {
                    fontSize: 14.0,
                    textColor: (0u8, 0, 0, 255),
                    margin: (0.0, 0.0, 5.0, 0.0),
                }
                
                // 动态大小和颜色的 View
                View {
                    width: {dynamic_size.get()},
                    height: {dynamic_size.get()},
                    color: {dynamic_color.get()},
                    borderRadius: 8.0,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,
                    
                    Text("Dynamic") {
                        fontSize: 10.0,
                        textColor: (255u8, 255, 255, 255),
                    }
                }
                
                // 控制按钮
                View {
                    width: "90%",
                    height: 30.0,
                    flexDirection: FlexDirection::Row,
                    justifyContent: JustifyContent::SpaceAround,
                    margin: (5.0, 0.0, 0.0, 0.0),
                    
                    View {
                        width: 60.0,
                        height: 24.0,
                        color: (60u32, 140, 240, 255),
                        borderRadius: 6.0,
                        justifyContent: JustifyContent::Center,
                        alignItems: AlignItems::Center,
                        onTap: {
                            let size = dynamic_size.clone();
                            move |_| size.set((size.get() + 10.0).min(100.0))
                        },
                        
                        Text("Size+") {
                            fontSize: 10.0,
                            textColor: (255u8, 255, 255, 255),
                        }
                    }
                    
                    View {
                        width: 60.0,
                        height: 24.0,
                        color: (240u32, 100, 60, 255),
                        borderRadius: 6.0,
                        justifyContent: JustifyContent::Center,
                        alignItems: AlignItems::Center,
                        onTap: {
                            let size = dynamic_size.clone();
                            move |_| size.set((size.get() - 10.0).max(30.0))
                        },
                        
                        Text("Size-") {
                            fontSize: 10.0,
                            textColor: (255u8, 255, 255, 255),
                        }
                    }
                    
                    View {
                        width: 60.0,
                        height: 24.0,
                        color: (140u32, 60, 240, 255),
                        borderRadius: 6.0,
                        justifyContent: JustifyContent::Center,
                        alignItems: AlignItems::Center,
                        onTap: {
                            let color = dynamic_color.clone();
                            let colors = vec![
                                (100u32, 100, 200, 255), // 蓝
                                (200u32, 100, 100, 255), // 红
                                (100u32, 200, 100, 255), // 绿
                                (200u32, 200, 100, 255), // 黄
                            ];
                            move |_| {
                                let current = color.get();
                                let idx = colors.iter().position(|&c| c == current).unwrap_or(0);
                                color.set(colors[(idx + 1) % colors.len()]);
                            }
                        },
                        
                        Text("Color") {
                            fontSize: 10.0,
                            textColor: (255u8, 255, 255, 255),
                        }
                    }
                }
            }
            
            // 事件日志
            View {
                width: "95%",
                flexGrow: 1.0,
                color: (25u32, 25, 35, 255),
                borderRadius: 8.0,
                padding: (10.0, 10.0, 10.0, 10.0),
                flexDirection: FlexDirection::Column,
                
                View {
                    width: "100%",
                    flexDirection: FlexDirection::Row,
                    justifyContent: JustifyContent::SpaceBetween,
                    margin: (0.0, 0.0, 10.0, 0.0),
                    
                    Text("Event Log") {
                        fontSize: 14.0,
                        textColor: (0u8, 0, 0, 255),
                    }
                    
                    // 清除按钮
                    View {
                        width: 50.0,
                        height: 24.0,
                        color: (100u32, 100, 100, 255),
                        borderRadius: 6.0,
                        justifyContent: JustifyContent::Center,
                        alignItems: AlignItems::Center,
                        onTap: {
                            let log = event_log.clone();
                            move |_| log.set("Cleared".to_string())
                        },
                        
                        Text("Clear") {
                            fontSize: 11.0,
                            textColor: (255u8, 255, 255, 255),
                        }
                    }
                }
                
                Text("{event_log}") {
                    fontSize: 12.0,
                    textColor: (0u8, 0, 0, 255),
                }
            }
        }
    }
}
