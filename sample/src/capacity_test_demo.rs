// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Capacity Test Demo - 测试动态扩容和代际ID

use dyxel_app::prelude::*;

#[app]
pub fn CapacityTestDemo() -> impl BaseView {
    // 状态
    let node_count = use_state(|| 0u32);
    let capacity = use_state(|| 256u32);  // 初始容量
    let generations = use_state(|| Vec::<u32>::new());
    
    // 克隆给闭包
    let node_count2 = node_count.clone();
    let capacity2 = capacity.clone();
    let generations2 = generations.clone();
    
    let add_nodes = move || {
        let current = node_count2.get();
        let batch = 50u32;  // 每次添加50个节点
        
        // 模拟批量创建节点
        let new_count = current + batch;
        node_count2.set(new_count);
        
        // 更新显示的容量（模拟Host端的扩容）
        if new_count > 200 && capacity2.get() == 256 {
            capacity2.set(512);
        } else if new_count > 450 && capacity2.get() == 512 {
            capacity2.set(1024);
        } else if new_count > 950 && capacity2.get() == 1024 {
            capacity2.set(2048);
        }
    };
    
    let node_count3 = node_count.clone();
    let capacity3 = capacity.clone();
    let generations3 = generations.clone();
    
    let remove_nodes = move || {
        let current = node_count3.get();
        if current >= 50 {
            let new_count = current - 50;
            node_count3.set(new_count);
            
            // 记录代际增加（模拟ID复用）
            let mut gens = generations3.get();
            gens.push(current);  // 记录被删除的批次
            generations3.set(gens);
        }
    };
    
    let reset = move || {
        node_count.set(0);
        capacity.set(256);
        generations.set(Vec::new());
    };
    
    rsx! {
        View {
            width: "100%",
            height: "100%",
            color: (20, 20, 30),
            flexDirection: FlexDirection::Column,
            alignItems: AlignItems::Center,
            padding: (20.0, 20.0, 20.0, 20.0),
            
            // 标题
            Text("Capacity Test") {
                fontSize: 24.0,
                textColor: (255, 255, 255, 255),
                margin: (0.0, 0.0, 20.0, 0.0),
            }
            
            // 统计信息
            View {
                width: "90%",
                height: 150.0,
                color: (40, 40, 60),
                borderRadius: 12.0,
                flexDirection: FlexDirection::Column,
                justifyContent: JustifyContent::Center,
                alignItems: AlignItems::Center,
                margin: (0.0, 0.0, 20.0, 0.0),
                
                Text("Active Nodes: {node_count}") {
                    fontSize: 20.0,
                    textColor: (100, 200, 255, 255),
                    margin: (0.0, 0.0, 10.0, 0.0),
                }
                
                Text("Capacity: {capacity}") {
                    fontSize: 16.0,
                    textColor: (255, 255, 255, 200),
                    margin: (0.0, 0.0, 10.0, 0.0),
                }
                
                Text("Usage: {((node_count as f32 / capacity as f32) * 100.0) as u32}%") {
                    fontSize: 16.0,
                    textColor: (255, 200, 100, 255),
                }
            }
            
            // 操作按钮
            View {
                width: "100%",
                flexDirection: FlexDirection::Row,
                justifyContent: JustifyContent::SpaceAround,
                margin: (0.0, 0.0, 20.0, 0.0),
                
                View {
                    width: 100.0,
                    height: 60.0,
                    color: (60, 140, 240),
                    borderRadius: 8.0,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,
                    onTap: move |_| add_nodes(),
                    
                    Text("+50") {
                        fontSize: 20.0,
                        textColor: (255, 255, 255, 255),
                    }
                }
                
                View {
                    width: 100.0,
                    height: 60.0,
                    color: (240, 100, 60),
                    borderRadius: 8.0,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,
                    onTap: move |_| remove_nodes(),
                    
                    Text("-50") {
                        fontSize: 20.0,
                        textColor: (255, 255, 255, 255),
                    }
                }
                
                View {
                    width: 100.0,
                    height: 60.0,
                    color: (100, 100, 100),
                    borderRadius: 8.0,
                    justifyContent: JustifyContent::Center,
                    alignItems: AlignItems::Center,
                    onTap: move |_| reset(),
                    
                    Text("Reset") {
                        fontSize: 20.0,
                        textColor: (255, 255, 255, 255),
                    }
                }
            }
            
            // 说明文字
            Text("Test: Click +50 to add nodes, watch capacity expand at 80% threshold") {
                fontSize: 12.0,
                textColor: (200, 200, 200, 200),
                textAlign: TextAlign::Center,
            }
            
            Text("Test: Click -50 to remove nodes, IDs will be recycled") {
                fontSize: 12.0,
                textColor: (200, 200, 200, 200),
                textAlign: TextAlign::Center,
            }
        }
    }
}
