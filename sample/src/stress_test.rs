// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Stress Test - 压测代际ID和动态扩容

use dyxel_app::prelude::*;

fn calc_capacity(nodes: u32) -> u32 {
    if nodes <= 200 { 256 }
    else if nodes <= 450 { 512 }
    else if nodes <= 950 { 1024 }
    else if nodes <= 1950 { 2048 }
    else { 4096 }
}

#[app]
pub fn StressTest() -> impl BaseView {
    // 压测状态
    let node_count = use_state(|| 0u32);
    let max_capacity = use_state(|| 256u32);
    let target_count = use_state(|| 100u32);
    let batch_size = use_state(|| 50u32);
    
    // 统计
    let total_created = use_state(|| 0u64);
    let total_deleted = use_state(|| 0u64);
    let peak_memory = use_state(|| 0u32);
    let expansion_log = use_state(|| String::new());
    
    // 克隆给闭包
    let node_count_add = node_count.clone();
    let max_capacity_add = max_capacity.clone();
    let target_count_add = target_count.clone();
    let batch_size_add = batch_size.clone();
    let total_created_add = total_created.clone();
    let peak_memory_add = peak_memory.clone();
    let expansion_log_add = expansion_log.clone();
    
    // 批量添加节点
    let add_batch = move || {
        let current = node_count_add.get();
        let batch = batch_size_add.get();
        let target = target_count_add.get();
        
        let new_count = (current + batch).min(target);
        let added = new_count - current;
        
        node_count_add.set(new_count);
        
        // 更新总数
        let prev_total = total_created_add.get();
        total_created_add.set(prev_total + added as u64);
        
        // 更新峰值
        let current_peak = peak_memory_add.get();
        if new_count > current_peak {
            peak_memory_add.set(new_count);
        }
        
        // 检测扩容事件
        let old_capacity = calc_capacity(current);
        let new_capacity = calc_capacity(new_count);
        if new_capacity > old_capacity {
            max_capacity_add.set(new_capacity);
            let log = expansion_log_add.get();
            let new_log = format!("{}Expand: {} -> {} (at {} nodes)\n", 
                log, old_capacity, new_capacity, new_count);
            expansion_log_add.set(new_log);
        }
    };
    
    // 批量删除
    let node_count_del = node_count.clone();
    let batch_size_del = batch_size.clone();
    let total_deleted_del = total_deleted.clone();
    
    let remove_batch = move || {
        let current = node_count_del.get();
        let batch = batch_size_del.get();
        
        let new_count = if current >= batch { current - batch } else { 0 };
        let removed = current - new_count;
        
        node_count_del.set(new_count);
        
        let prev_total = total_deleted_del.get();
        total_deleted_del.set(prev_total + removed as u64);
    };
    
    // 清空
    let node_count_clear = node_count.clone();
    let total_deleted_clear = total_deleted.clone();
    
    let clear_all = move || {
        let current = node_count_clear.get();
        let prev_total = total_deleted_clear.get();
        total_deleted_clear.set(prev_total + current as u64);
        node_count_clear.set(0);
    };
    
    // 调整目标 - 增加
    let target_count_adj_inc = target_count.clone();
    let adjust_target_inc = move || {
        let current = target_count_adj_inc.get();
        let new_target = (current + 100).min(4096);
        target_count_adj_inc.set(new_target.max(10));
    };
    
    // 调整目标 - 减少
    let target_count_adj_dec = target_count.clone();
    let adjust_target_dec = move || {
        let current = target_count_adj_dec.get();
        let new_target = current.saturating_sub(100).max(10);
        target_count_adj_dec.set(new_target);
    };
    
    // 调整批次 - 增加
    let batch_size_adj_inc = batch_size.clone();
    let adjust_batch_inc = move || {
        let current = batch_size_adj_inc.get();
        let new_batch = (current + 10).min(500);
        batch_size_adj_inc.set(new_batch);
    };
    
    // 调整批次 - 减少
    let batch_size_adj_dec = batch_size.clone();
    let adjust_batch_dec = move || {
        let current = batch_size_adj_dec.get();
        let new_batch = current.saturating_sub(10).max(10);
        batch_size_adj_dec.set(new_batch);
    };
    
    rsx! {
        View {
            width: "100%",
            height: "100%",
            color: (15u32, 15, 25),
            flexDirection: FlexDirection::Column,
            alignItems: AlignItems::Center,
            padding: (10.0, 10.0, 10.0, 10.0),
            
            // 标题
            Text("Stress Test") {
                fontSize: 22.0,
                textColor: (0u8, 0, 0, 255),
                margin: (0.0, 0.0, 10.0, 0.0),
            }
            
            // 主统计面板
            View {
                width: "95%",
                height: 160.0,
                color: (30u32, 30, 45),
                borderRadius: 12.0,
                flexDirection: FlexDirection::Column,
                justifyContent: JustifyContent::Center,
                alignItems: AlignItems::Center,
                margin: (0.0, 0.0, 10.0, 0.0),
                
                // 节点数量显示
                View {
                    flexDirection: FlexDirection::Row,
                    alignItems: AlignItems::Center,
                    margin: (0.0, 0.0, 5.0, 0.0),
                    
                    Text("{node_count}") {
                        fontSize: 28.0,
                        fontWeight: 700,
                        textColor: (0u8, 0, 0, 255),
                    }
                    
                    Text(" / {max_capacity} nodes") {
                        fontSize: 18.0,
                        textColor: (0u8, 0, 0, 255),
                    }
                }
                
                // 使用率条背景
                View {
                    width: 250.0,
                    height: 20.0,
                    color: (50u32, 50, 70),
                    borderRadius: 10.0,
                    margin: (10.0, 0.0, 10.0, 0.0),
                    
                    // 使用率条前景 - 使用固定颜色，避免代码块类型推断问题
                    // 使用率条前景（内联定义）
                    View {
                        width: {
                            let n = node_count.get();
                            let c = calc_capacity(n);
                            250.0 * ((n as f32 / c as f32).min(1.0))
                        },
                        height: 20.0,
                        color: {
                            let n = node_count.get();
                            let c = calc_capacity(n);
                            let pct = n as f32 / c as f32;
                            if pct < 0.5 { (100u32, 255, 100) }
                            else if pct < 0.8 { (255u32, 255, 100) }
                            else { (255u32, 100, 100) }
                        },
                        borderRadius: 10.0,
                    }
                }
                
                // 详细统计
                View {
                    width: "90%",
                    flexDirection: FlexDirection::Row,
                    justifyContent: JustifyContent::SpaceAround,
                    margin: (10.0, 0.0, 0.0, 0.0),
                    
                    // Created
                    View {
                        flexDirection: FlexDirection::Column,
                        alignItems: AlignItems::Center,
                        
                        Text("{total_created}") {
                            fontSize: 16.0,
                            fontWeight: 600,
                            textColor: (0u8, 0, 0, 255),
                        }
                        Text("Created") {
                            fontSize: 10.0,
                            textColor: (0u8, 0, 0, 255),
                        }
                    }
                    
                    // Deleted
                    View {
                        flexDirection: FlexDirection::Column,
                        alignItems: AlignItems::Center,
                        
                        Text("{total_deleted}") {
                            fontSize: 16.0,
                            fontWeight: 600,
                            textColor: (0u8, 0, 0, 255),
                        }
                        Text("Deleted") {
                            fontSize: 10.0,
                            textColor: (0u8, 0, 0, 255),
                        }
                    }
                    
                    // Peak
                    View {
                        flexDirection: FlexDirection::Column,
                        alignItems: AlignItems::Center,
                        
                        Text("{peak_memory}") {
                            fontSize: 16.0,
                            fontWeight: 600,
                            textColor: (0u8, 0, 0, 255),
                        }
                        Text("Peak") {
                            fontSize: 10.0,
                            textColor: (0u8, 0, 0, 255),
                        }
                    }
                }
            }
            
            // 扩容事件日志
            View {
                width: "95%",
                height: 80.0,
                color: (25u32, 25, 35),
                borderRadius: 8.0,
                margin: (0.0, 0.0, 10.0, 0.0),
                flexDirection: FlexDirection::Column,
                
                Text("Expansion Events:") {
                    fontSize: 12.0,
                    textColor: (0u8, 0, 0, 255),
                    margin: (5.0, 5.0, 5.0, 5.0),
                }
                
                Text("{expansion_log}") {
                    fontSize: 10.0,
                    textColor: (0u8, 0, 0, 255),
                    margin: (5.0, 0.0, 5.0, 5.0),
                }
            }
            
            // 控制面板
            View {
                width: "95%",
                height: 180.0,
                color: (35u32, 35, 50),
                borderRadius: 12.0,
                flexDirection: FlexDirection::Column,
                padding: (10.0, 10.0, 10.0, 10.0),
                margin: (0.0, 0.0, 10.0, 0.0),
                
                // 目标设置
                View {
                    width: "100%",
                    height: 40.0,
                    flexDirection: FlexDirection::Row,
                    alignItems: AlignItems::Center,
                    justifyContent: JustifyContent::SpaceBetween,
                    margin: (0.0, 0.0, 5.0, 0.0),
                    
                    Text("Target:") {
                        fontSize: 14.0,
                        textColor: (0u8, 0, 0, 255),
                        width: 60.0,
                    }
                    
                    View {
                        width: 40.0,
                        height: 30.0,
                        color: (80u32, 80, 100),
                        borderRadius: 6.0,
                        justifyContent: JustifyContent::Center,
                        alignItems: AlignItems::Center,
                        onTap: move |_| adjust_target_dec(),
                        
                        Text("-") {
                            fontSize: 18.0,
                            textColor: (255u8, 255, 255, 255),
                        }
                    }
                    
                    Text("{target_count}") {
                        fontSize: 16.0,
                        fontWeight: 600,
                        textColor: (0u8, 0, 0, 255),
                        width: 80.0,
                    }
                    
                    View {
                        width: 40.0,
                        height: 30.0,
                        color: (80u32, 80, 100),
                        borderRadius: 6.0,
                        justifyContent: JustifyContent::Center,
                        alignItems: AlignItems::Center,
                        onTap: move |_| adjust_target_inc(),
                        
                        Text("+") {
                            fontSize: 18.0,
                            textColor: (255u8, 255, 255, 255),
                        }
                    }
                }
                
                // 批次设置
                View {
                    width: "100%",
                    height: 40.0,
                    flexDirection: FlexDirection::Row,
                    alignItems: AlignItems::Center,
                    justifyContent: JustifyContent::SpaceBetween,
                    margin: (0.0, 0.0, 5.0, 0.0),
                    
                    Text("Batch:") {
                        fontSize: 14.0,
                        textColor: (0u8, 0, 0, 255),
                        width: 60.0,
                    }
                    
                    View {
                        width: 40.0,
                        height: 30.0,
                        color: (80u32, 80, 100),
                        borderRadius: 6.0,
                        justifyContent: JustifyContent::Center,
                        alignItems: AlignItems::Center,
                        onTap: move |_| adjust_batch_dec(),
                        
                        Text("-") {
                            fontSize: 18.0,
                            textColor: (255u8, 255, 255, 255),
                        }
                    }
                    
                    Text("{batch_size}") {
                        fontSize: 16.0,
                        fontWeight: 600,
                        textColor: (0u8, 0, 0, 255),
                        width: 80.0,
                    }
                    
                    View {
                        width: 40.0,
                        height: 30.0,
                        color: (80u32, 80, 100),
                        borderRadius: 6.0,
                        justifyContent: JustifyContent::Center,
                        alignItems: AlignItems::Center,
                        onTap: move |_| adjust_batch_inc(),
                        
                        Text("+") {
                            fontSize: 18.0,
                            textColor: (255u8, 255, 255, 255),
                        }
                    }
                }
                
                // 操作按钮
                View {
                    width: "100%",
                    flexDirection: FlexDirection::Row,
                    justifyContent: JustifyContent::SpaceAround,
                    margin: (10.0, 0.0, 0.0, 0.0),
                    
                    // +Batch 按钮
                    View {
                        width: 80.0,
                        height: 45.0,
                        color: (60u32, 140, 240),
                        borderRadius: 8.0,
                        justifyContent: JustifyContent::Center,
                        alignItems: AlignItems::Center,
                        onTap: move |_| add_batch(),
                        
                        Text("+Batch") {
                            fontSize: 14.0,
                            fontWeight: 600,
                            textColor: (255u8, 255, 255, 255),
                        }
                    }
                    
                    // -Batch 按钮
                    View {
                        width: 80.0,
                        height: 45.0,
                        color: (240u32, 100, 60),
                        borderRadius: 8.0,
                        justifyContent: JustifyContent::Center,
                        alignItems: AlignItems::Center,
                        onTap: move |_| remove_batch(),
                        
                        Text("-Batch") {
                            fontSize: 14.0,
                            fontWeight: 600,
                            textColor: (255u8, 255, 255, 255),
                        }
                    }
                    
                    // Clear 按钮
                    View {
                        width: 80.0,
                        height: 45.0,
                        color: (100u32, 100, 100),
                        borderRadius: 8.0,
                        justifyContent: JustifyContent::Center,
                        alignItems: AlignItems::Center,
                        onTap: move |_| clear_all(),
                        
                        Text("Clear") {
                            fontSize: 14.0,
                            fontWeight: 600,
                            textColor: (255u8, 255, 255, 255),
                        }
                    }
                }
            }
            
            // 说明
            Text("Tap +Batch to add nodes, watch capacity expand at thresholds") {
                fontSize: 11.0,
                textColor: (0u8, 0, 0, 255),
                margin: (5.0, 0.0, 0.0, 0.0),
            }
        }
    }
}
