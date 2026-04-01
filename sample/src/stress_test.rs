// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Stress Test - 压测代际ID和动态扩容

use dyxel_app::prelude::*;

#[app]
pub fn StressTest() -> impl BaseView {
    // 压测状态
    let node_count = use_state(|| 0u32);
    let target_count = use_state(|| 100u32);
    let batch_size = use_state(|| 50u32);
    let stress_mode = use_state(|| StressMode::None);
    
    // 统计
    let total_created = use_state(|| 0u64);
    let total_deleted = use_state(|| 0u64);
    let peak_memory = use_state(|| 0u32);  // 模拟峰值节点数
    let expansion_events = use_state(|| Vec::<String>::new());
    
    // 性能监控
    let last_fps = use_state(|| 60.0f32);
    let frame_drops = use_state(|| 0u32);
    
    // 克隆给闭包
    let node_count2 = node_count.clone();
    let target_count2 = target_count.clone();
    let batch_size2 = batch_size.clone();
    let stress_mode2 = stress_mode.clone();
    let total_created2 = total_created.clone();
    let total_deleted2 = total_deleted.clone();
    let peak_memory2 = peak_memory.clone();
    let expansion_events2 = expansion_events.clone();
    
    // 批量添加节点
    let add_batch = move || {
        let current = node_count2.get();
        let batch = batch_size2.get();
        let target = target_count2.get();
        
        let new_count = (current + batch).min(target);
        let added = new_count - current;
        
        node_count2.set(new_count);
        total_created2.update(|t| *t + added as u64);
        
        // 更新峰值
        let current_peak = peak_memory2.get();
        if new_count > current_peak {
            peak_memory2.set(new_count);
        }
        
        // 检测扩容事件（模拟Host端逻辑）
        let capacity = if new_count <= 200 { 256 }
            else if new_count <= 450 { 512 }
            else if new_count <= 950 { 1024 }
            else if new_count <= 1950 { 2048 }
            else { 4096 };
        
        if capacity > get_current_capacity(current) {
            let mut events = expansion_events2.get();
            events.push(format!("{} → {} (nodes: {})", 
                get_current_capacity(current), capacity, new_count));
            expansion_events2.set(events);
        }
    };
    
    // 批量删除节点
    let node_count3 = node_count.clone();
    let batch_size3 = batch_size.clone();
    let total_deleted3 = total_deleted.clone();
    
    let remove_batch = move || {
        let current = node_count3.get();
        let batch = batch_size3.get();
        
        let new_count = if current >= batch { current - batch } else { 0 };
        let removed = current - new_count;
        
        node_count3.set(new_count);
        total_deleted3.update(|t| *t + removed as u64);
    };
    
    // 清空所有
    let node_count4 = node_count.clone();
    let total_deleted4 = total_deleted.clone();
    
    let clear_all = move || {
        let current = node_count4.get();
        total_deleted4.update(|t| *t + current as u64);
        node_count4.set(0);
    };
    
    // 快速压测：创建到目标然后清空，循环
    let node_count5 = node_count.clone();
    let target_count3 = target_count.clone();
    let stress_mode3 = stress_mode.clone();
    let total_created3 = total_created.clone();
    let total_deleted5 = total_deleted.clone();
    let expansion_events3 = expansion_events.clone();
    
    let toggle_stress_test = move || {
        let mode = stress_mode3.get();
        match mode {
            StressMode::None => {
                stress_mode3.set(StressMode::Running);
                // 启动压测循环
                spawn_local(async move {
                    loop {
                        let mode = stress_mode3.get();
                        if mode == StressMode::None {
                            break;
                        }
                        
                        let target = target_count3.get();
                        let current = node_count5.get();
                        
                        if current < target {
                            // 快速创建
                            let batch = (target - current).min(100);
                            node_count5.update(|n| *n + batch);
                            total_created3.update(|t| *t + batch as u64);
                            
                            // 检测扩容
                            let capacity = get_current_capacity(current + batch);
                            if capacity > get_current_capacity(current) {
                                let mut events = expansion_events3.get();
                                if events.len() < 10 {  // 限制显示数量
                                    events.push(format!("Expand to {} at {}", capacity, current + batch));
                                    expansion_events3.set(events);
                                }
                            }
                        } else {
                            // 清空，模拟ID复用
                            total_deleted5.update(|t| *t + current as u64);
                            node_count5.set(0);
                        }
                        
                        // 16ms 延迟（约60fps）
                        sleep(16).await;
                    }
                });
            }
            _ => {
                stress_mode3.set(StressMode::None);
            }
        }
    };
    
    // 调整目标
    let target_count4 = target_count.clone();
    let adjust_target = move |delta: i32| {
        let current = target_count4.get();
        let new_target = if delta > 0 {
            (current + delta as u32).min(4096)
        } else {
            current.saturating_sub((-delta) as u32)
        };
        target_count4.set(new_target.max(10));
    };
    
    // 调整批次
    let batch_size4 = batch_size.clone();
    let adjust_batch = move |delta: i32| {
        let current = batch_size4.get();
        let new_batch = if delta > 0 {
            (current + delta as u32).min(500)
        } else {
            current.saturating_sub((-delta) as u32)
        };
        batch_size4.set(new_batch.max(10));
    };
    
    rsx! {
        View {
            width: "100%",
            height: "100%",
            color: (15, 15, 25),
            flexDirection: FlexDirection::Column,
            alignItems: AlignItems::Center,
            padding: (10.0, 10.0, 10.0, 10.0),
            
            // 标题
            Text("🔥 Stress Test 🔥") {
                fontSize: 22.0,
                textColor: (255, 100, 100, 255),
                margin: (0.0, 0.0, 10.0, 0.0),
            }
            
            // 主统计面板
            View {
                width: "95%",
                height: 180.0,
                color: (30, 30, 45),
                borderRadius: 12.0,
                flexDirection: FlexDirection::Column,
                justifyContent: JustifyContent::Center,
                alignItems: AlignItems::Center,
                margin: (0.0, 0.0, 10.0, 0.0),
                
                // 节点数量大字体
                Text("{node_count} / {get_current_capacity(node_count)} nodes") {
                    fontSize: 32.0,
                    fontWeight: 700,
                    textColor: (100, 200, 255, 255),
                    margin: (0.0, 0.0, 5.0, 0.0),
                }
                
                // 使用率条
                View {
                    width: 250.0,
                    height: 20.0,
                    color: (50, 50, 70),
                    borderRadius: 10.0,
                    margin: (10.0, 0.0, 10.0, 0.0),
                    
                    View {
                        width: get_usage_width(node_count, get_current_capacity(node_count)),
                        height: 20.0,
                        color: get_usage_color(node_count, get_current_capacity(node_count)),
                        borderRadius: 10.0,
                    }
                }
                
                // 详细统计
                View {
                    width: "90%",
                    flexDirection: FlexDirection::Row,
                    justifyContent: JustifyContent::SpaceAround,
                    margin: (10.0, 0.0, 0.0, 0.0),
                    
                    StatItem("Created", format!("{}", total_created.get()), (100, 255, 100))
                    StatItem("Deleted", format!("{}", total_deleted.get()), (255, 100, 100))
                    StatItem("Peak", format!("{}", peak_memory.get()), (255, 200, 100))
                }
            }
            
            // 扩容事件日志
            View {
                width: "95%",
                height: 80.0,
                color: (25, 25, 35),
                borderRadius: 8.0,
                margin: (0.0, 0.0, 10.0, 0.0),
                overflow: Overflow::Hidden,
                
                Text("Expansion Events:") {
                    fontSize: 12.0,
                    textColor: (150, 150, 150, 255),
                    margin: (5.0, 5.0, 5.0, 5.0),
                }
                
                View {
                    width: "100%",
                    flexGrow: 1.0,
                    flexDirection: FlexDirection::Column,
                    overflow: Overflow::Scroll,
                    
                    for (i, event) in expansion_events.get().iter().enumerate() {
                        Text("{}. {}", i + 1, event) {
                            fontSize: 10.0,
                            textColor: (200, 200, 100, 255),
                            margin: (2.0, 0.0, 2.0, 5.0),
                        }
                    }
                }
            }
            
            // 控制面板
            View {
                width: "95%",
                height: 200.0,
                color: (35, 35, 50),
                borderRadius: 12.0,
                flexDirection: FlexDirection::Column,
                padding: (10.0, 10.0, 10.0, 10.0),
                margin: (0.0, 0.0, 10.0, 0.0),
                
                // 目标设置
                ControlRow("Target:", target_count.get().to_string(), 
                    move |_| adjust_target(-100), 
                    move |_| adjust_target(100))
                
                // 批次设置
                ControlRow("Batch:", batch_size.get().to_string(),
                    move |_| adjust_batch(-10),
                    move |_| adjust_batch(10))
                
                // 操作按钮
                View {
                    width: "100%",
                    flexDirection: FlexDirection::Row,
                    justifyContent: JustifyContent::SpaceAround,
                    margin: (10.0, 0.0, 0.0, 0.0),
                    
                    ActionButton("+Batch", (60, 140, 240), move |_| add_batch())
                    ActionButton("-Batch", (240, 100, 60), move |_| remove_batch())
                    ActionButton("Clear", (100, 100, 100), move |_| clear_all())
                }
            }
            
            // 压测控制
            View {
                width: "95%",
                height: 70.0,
                color: if stress_mode.get() == StressMode::Running { 
                    (60, 30, 30)  // 红色背景表示运行中
                } else { 
                    (30, 60, 30)  // 绿色背景表示停止
                },
                borderRadius: 12.0,
                justifyContent: JustifyContent::Center,
                alignItems: AlignItems::Center,
                onTap: move |_| toggle_stress_test(),
                
                Text(if stress_mode.get() == StressMode::Running { 
                    "⏹ STOP STRESS TEST" 
                } else { 
                    "▶ START STRESS TEST" 
                }) {
                    fontSize: 18.0,
                    fontWeight: 700,
                    textColor: (255, 255, 255, 255),
                }
            }
            
            // 说明
            Text("Stress: rapid create/delete cycles to test ID recycling") {
                fontSize: 10.0,
                textColor: (150, 150, 150, 200),
                margin: (10.0, 0.0, 0.0, 0.0),
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StressMode {
    None,
    Running,
}

fn get_current_capacity(nodes: u32) -> u32 {
    if nodes <= 200 { 256 }
    else if nodes <= 450 { 512 }
    else if nodes <= 950 { 1024 }
    else if nodes <= 1950 { 2048 }
    else { 4096 }
}

fn get_usage_width(nodes: u32, capacity: u32) -> f32 {
    let pct = (nodes as f32 / capacity as f32).min(1.0);
    250.0 * pct
}

fn get_usage_color(nodes: u32, capacity: u32) -> (u8, u8, u8) {
    let pct = nodes as f32 / capacity as f32;
    if pct < 0.5 { (100, 255, 100) }      // 绿色
    else if pct < 0.8 { (255, 255, 100) } // 黄色
    else { (255, 100, 100) }              // 红色
}

// 辅助组件
fn StatItem(label: &'static str, value: String, color: (u8, u8, u8)) -> impl BaseView {
    rsx! {
        View {
            flexDirection: FlexDirection::Column,
            alignItems: AlignItems::Center,
            
            Text("{value}") {
                fontSize: 16.0,
                fontWeight: 600,
                textColor: (color.0, color.1, color.2, 255),
            }
            Text("{label}") {
                fontSize: 10.0,
                textColor: (150, 150, 150, 255),
            }
        }
    }
}

fn ControlRow(
    label: &'static str, 
    value: String,
    on_minus: impl FnMut() + 'static,
    on_plus: impl FnMut() + 'static,
) -> impl BaseView {
    rsx! {
        View {
            width: "100%",
            height: 40.0,
            flexDirection: FlexDirection::Row,
            alignItems: AlignItems::Center,
            justifyContent: JustifyContent::SpaceBetween,
            margin: (0.0, 0.0, 5.0, 0.0),
            
            Text("{label}") {
                fontSize: 14.0,
                textColor: (200, 200, 200, 255),
                width: 60.0,
            }
            
            View {
                width: 40.0,
                height: 30.0,
                color: (80, 80, 100),
                borderRadius: 6.0,
                justifyContent: JustifyContent::Center,
                alignItems: AlignItems::Center,
                onTap: move |_| on_minus(),
                
                Text("-") {
                    fontSize: 18.0,
                    textColor: (255, 255, 255, 255),
                }
            }
            
            Text("{value}") {
                fontSize: 16.0,
                fontWeight: 600,
                textColor: (255, 255, 255, 255),
                width: 80.0,
                textAlign: TextAlign::Center,
            }
            
            View {
                width: 40.0,
                height: 30.0,
                color: (80, 80, 100),
                borderRadius: 6.0,
                justifyContent: JustifyContent::Center,
                alignItems: AlignItems::Center,
                onTap: move |_| on_plus(),
                
                Text("+") {
                    fontSize: 18.0,
                    textColor: (255, 255, 255, 255),
                }
            }
        }
    }
}

fn ActionButton(
    label: &'static str, 
    color: (u8, u8, u8),
    on_tap: impl FnMut() + 'static,
) -> impl BaseView {
    rsx! {
        View {
            width: 80.0,
            height: 45.0,
            color: color,
            borderRadius: 8.0,
            justifyContent: JustifyContent::Center,
            alignItems: AlignItems::Center,
            onTap: move |_| on_tap(),
            
            Text("{label}") {
                fontSize: 14.0,
                fontWeight: 600,
                textColor: (255, 255, 255, 255),
            }
        }
    }
}

// 模拟 sleep 函数
fn sleep(ms: u32) -> impl Future<Output = ()> {
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    
    struct SleepFuture {
        target: std::time::Instant,
    }
    
    impl Future for SleepFuture {
        type Output = ();
        
        fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
            if std::time::Instant::now() >= self.target {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }
    }
    
    SleepFuture {
        target: std::time::Instant::now() + std::time::Duration::from_millis(ms as u64),
    }
}
