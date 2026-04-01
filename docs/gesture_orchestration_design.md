# Dyxel 手势编排系统设计 (Gesture Orchestration System)

## 1. 设计目标

- **统一手势模型**: 单一手势与组合手势使用统一的 API 和内部表示
- **灵活编排**: 支持顺序、并行、互斥三种组合策略
- **精确控制**: 提供手势裁决、优先级、动态启闭等干预机制
- **可扩展性**: 易于添加新手势类型和组合策略

---

## 2. 核心架构

### 2.1 类型层级

```
Gesture (trait)
├── SingleGesture
│   ├── TapGesture { count: u32, max_duration_ms: u64 }
│   ├── LongPressGesture { duration_ms: u64 }
│   ├── PanGesture { direction: PanDirection, min_distance: f32 }
│   ├── PinchGesture { min_scale: f32 }
│   ├── RotationGesture { min_angle: f32 }
│   └── SwipeGesture { direction: SwipeDirection, min_velocity: f32 }
│
└── CompositeGesture
    ├── SequenceGesture { steps: Vec<Gesture> }
    ├── ParallelGesture { gestures: Vec<Gesture> }
    └── ExclusiveGesture { candidates: Vec<Gesture> }
```

### 2.2 手势识别器 (GestureRecognizer)

```rust
/// 手势识别器状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecognizerState {
    Possible,   // 可能识别
    Began,      // 开始识别
    Changed,    // 识别中(持续手势)
    Ended,      // 识别结束
    Failed,     // 识别失败
    Cancelled,  // 被取消
}

/// 手势识别器 trait
pub trait GestureRecognizer: Send + Sync {
    /// 处理原始输入事件
    fn handle_event(&mut self, event: &PointerEvent) -> Vec<GestureEvent>;
    
    /// 当前状态
    fn state(&self) -> RecognizerState;
    
    /// 目标节点
    fn target_node_id(&self) -> u32;
    
    /// 手势优先级 (数值越高优先级越高)
    fn priority(&self) -> i32 { 0 }
    
    /// 是否允许与其他手势并行
    fn can_parallel_with(&self, other: &dyn GestureRecognizer) -> bool { false }
    
    /// 裁决请求 - 当需要决定是否接受手势时调用
    fn on_judge(&self, context: &GestureContext) -> GestureJudgement {
        GestureJudgement::Accept
    }
    
    /// 重置识别器
    fn reset(&mut self);
}
```

---

## 3. 手势编排 DSL

### 3.1 基础手势配置（View 级语法糖）

```rust
// 单击手势 -> 内部映射为 TapGesture { count: 1, onGestureEnded: ... }
View {
    onTap: |event| { /* ... */ }
}

// 双击手势
View {
    onDoubleTap: |event| { /* ... */ }
}

// 长按手势 -> 内部映射为 LongPressGesture { onGestureEnded: ... }
View {
    onLongPress: |event| { /* ... */ }
}

// 滑动手势 -> 内部映射为 PanGesture { onGestureChanged: ... }
View {
    onPanUpdate: |event| { /* ... */ }
}
```

### 3.2 显式手势配置（完整形态）

**所有单一手势统一支持以下回调**：
- `onGestureBegan`: 状态从 `Possible` 变为 `Began` 时触发
- `onGestureChanged`: 持续手势（Pan/Pinch/Rotation）在状态变化时触发；离散手势（Tap/LongPress/Swipe）通常不触发
- `onGestureEnded`: 状态变为 `Ended` 时触发
- `onGestureCancelled`: 状态变为 `Cancelled` 时触发

```rust
// Tap 手势（离散手势）
View {
    gesture: TapGesture {
        count: 2,              // 双击
        max_duration_ms: 300,  // 两次点击间隔不超过300ms
        onGestureBegan: |event| {
            log.info("Double tap began at ({}, {})", event.x, event.y);
        },
        onGestureEnded: |event| {
            log.info("Double tap ended at ({}, {})", event.x, event.y);
        },
        onGestureCancelled: |event| {
            log.info("Double tap cancelled");
        }
    }
}

// LongPress 手势（离散手势）
View {
    gesture: LongPressGesture {
        duration_ms: 500,
        onGestureBegan: |event| {
            log.info("Long press began");
        },
        onGestureEnded: |event| {
            log.info("Long press ended");
        },
        onGestureCancelled: |event| {
            log.info("Long press cancelled");
        }
    }
}

// Pan 手势（持续手势）
View {
    gesture: PanGesture {
        direction: PanDirection::Horizontal,
        min_distance: 20.0,
        onGestureBegan: |event| { /* ... */ },
        onGestureChanged: |event| { /* ... */ },
        onGestureEnded: |event| { /* ... */ },
        onGestureCancelled: |event| { /* ... */ },
    }
}
```

### 3.3 组合手势

**组合手势本身只支持裁决回调**（`onGestureJudgeBegin` / `onGestureRecognizerJudgeBegin`），**子手势支持完整状态监听**：

```rust
// 顺序手势: 双击后长按
// 组合手势只负责编排，每个子手势有自己的状态回调
View {
    gesture: SequenceGesture([
        TapGesture { 
            count: 2,
            onGestureBegan: |event| { log.info("Step 1: Double tap began"); },
            onGestureEnded: |event| { log.info("Step 1: Double tap done"); }
        },
        LongPressGesture { 
            duration_ms: 500,
            onGestureBegan: |event| { log.info("Step 2: Long press began"); },
            onGestureEnded: |event| { log.info("Step 2: Long press done"); }
        }
    ]),
    onGestureJudgeBegin: |gesture, context| {
        // 只有组合手势级别支持裁决
        GestureJudgement::Accept
    }
}

// 并行手势: 同时检测缩放和旋转
View {
    gesture: ParallelGesture([
        PinchGesture {
            onGestureBegan: |event| { /* ... */ },
            onGestureChanged: |event| { update_scale(event.scale); },
            onGestureEnded: |event| { /* ... */ },
        },
        RotationGesture {
            onGestureBegan: |event| { /* ... */ },
            onGestureChanged: |event| { update_rotation(event.rotation); },
            onGestureEnded: |event| { /* ... */ },
        }
    ])
}

// 互斥手势: 单击或长按，只能触发一个
View {
    gesture: ExclusiveGesture([
        TapGesture { 
            count: 1,
            onGestureBegan: |event| { log.info("Tap began"); },
            onGestureEnded: |event| { log.info("Tap triggered"); }
        },
        LongPressGesture { 
            duration_ms: 500,
            onGestureBegan: |event| { log.info("Long press began"); },
            onGestureEnded: |event| { log.info("Long press triggered"); }
        }
    ])
}
```

---

## 4. 手势冲突处理

### 4.1 冲突解决策略

```rust
/// 冲突解决策略
pub enum ConflictResolution {
    /// 优先满足阈值的手势先响应
    FirstToSatisfy,
    
    /// 子组件手势优先
    ChildPriority,
    
    /// 父组件手势优先  
    ParentPriority,
    
    /// 按优先级字段比较
    ByPriority(i32),
    
    /// 自定义裁决
    Custom(Box<dyn Fn(&GestureContext) -> GestureJudgement>),
}

/// 手势裁决结果
pub enum GestureJudgement {
    Accept,     // 接受手势
    Reject,     // 拒绝手势
    Delay(u64), // 延迟裁决(毫秒)
}
```

### 4.2 手势裁决回调

```rust
// 基础裁决 - 在手势即将成功时拦截
// 适用于单一手势和组合手势
View {
    gesture: TapGesture { count: 1 },
    onGestureJudgeBegin: |gesture, context| {
        if gesture.gesture_type == GestureType::Tap {
            // 只在点击区域上半部分响应
            if context.location.y < context.bounds.height / 2.0 {
                GestureJudgement::Accept
            } else {
                GestureJudgement::Reject
            }
        } else {
            GestureJudgement::Accept
        }
    }
}

// 高级裁决 - 获取所有竞争手势进行裁决
// 适用于单一手势和组合手势
View {
    gesture: ExclusiveGesture([...]),
    onGestureRecognizerJudgeBegin: |recognizers, current, context| {
        // 检查是否有子组件的手势在竞争
        let has_child_competitor = recognizers.iter()
            .any(|r| r.target_node_id != current.target_node_id 
                     && r.state() == RecognizerState::Began);
        
        if has_child_competitor {
            // 让子组件优先
            GestureJudgement::Reject
        } else {
            GestureJudgement::Accept
        }
    }
}
```

### 4.3 并行动态控制

```rust
// 设置手势并行(父子组件同类型手势同时响应)
ScrollView {
    gesture: PanGesture::default(),
    shouldParallelWith: |child_gesture| {
        // 允许与内部 ScrollView 的 Pan 手势并行
        child_gesture.gesture_type == GestureType::Pan 
            && child_gesture.target.is::<ScrollView>()
    },
    
    // 动态控制手势开闭
    onGestureRecognizerJudgeBegin: |recognizers, current, context| {
        let scroll_state = context.get_scroll_state();
        
        // 如果已滚动到底部，允许父组件手势响应
        if scroll_state.at_bottom {
            current.set_enabled(true);
        } else {
            current.set_enabled(false);
        }
    }
}
```

---

## 5. 手势识别器管理

### 5.1 GestureArena 改进

```rust
/// 手势竞技场 - 管理同一指针的竞争手势
pub struct GestureArena {
    id: ArenaId,
    pointer_id: u32,
    members: Vec<ArenaMember>,
    state: ArenaState,
    
    // 新增: 组合手势协调器
    composite_coordinator: Option<Box<dyn CompositeCoordinator>>,
}

pub enum ArenaState {
    Open,       // 开放，可添加新成员
    Competing,  // 竞争中
    Resolved { winner: u32 }, // 已解决
    Delayed { until: Instant }, // 延迟关闭(等待多击手势)
}

/// 竞技场成员
struct ArenaMember {
    recognizer: Box<dyn GestureRecognizer>,
    state: MemberState,
    events: Vec<GestureEvent>,
}

enum MemberState {
    Pending,    // 等待中
    Active,     // 活跃(已开始识别)
    Accepted,   // 已接受
    Rejected,   // 已拒绝
}
```

### 5.2 组合手势协调器

```rust
/// 组合手势协调器 trait
pub trait CompositeCoordinator: Send + Sync {
    /// 协调多个手势的状态
    fn coordinate(&mut self, members: &mut [ArenaMember]) -> CoordinationResult;
    
    /// 检查是否完成
    fn is_complete(&self) -> bool;
    
    /// 检查是否失败
    fn has_failed(&self) -> bool;
}

/// 顺序手势协调器
pub struct SequenceCoordinator {
    current_step: usize,
    steps: Vec<Box<dyn GestureRecognizer>>,
}

impl CompositeCoordinator for SequenceCoordinator {
    fn coordinate(&mut self, members: &mut [ArenaMember]) {
        // 只有当前步骤的手势可以激活
        for (i, member) in members.iter_mut().enumerate() {
            if i != self.current_step {
                member.state = MemberState::Rejected;
            }
        }
        
        // 当前步骤完成，进入下一步
        if members[self.current_step].state == MemberState::Accepted {
            self.current_step += 1;
            if self.current_step >= self.steps.len() {
                // 全部完成
            }
        }
    }
}

/// 互斥手势协调器
pub struct ExclusiveCoordinator {
    winner_selected: bool,
}

impl CompositeCoordinator for ExclusiveCoordinator {
    fn coordinate(&mut self, members: &mut [ArenaMember]) {
        if self.winner_selected {
            return;
        }
        
        // 第一个成功的手势成为 winner
        if let Some(winner_idx) = members.iter()
            .position(|m| m.state == MemberState::Accepted) {
            
            // 拒绝其他所有手势
            for (i, member) in members.iter_mut().enumerate() {
                if i != winner_idx {
                    member.state = MemberState::Rejected;
                }
            }
            self.winner_selected = true;
        }
    }
}
```

---

## 6. 实现路线图

### Phase 1: 基础重构
1. 重构 `GestureRecognizer` trait，添加 `can_parallel_with` 和 `on_judge`
2. 修改 `TapGestureRecognizer`，支持 `count` 参数
3. 更新 Arena 支持 `Delayed` 状态(等待多击)

### Phase 2: 组合手势
1. 实现 `SequenceCoordinator`
2. 实现 `ParallelCoordinator` 
3. 实现 `ExclusiveCoordinator`
4. 添加 `CompositeGesture` DSL，子手势支持独立状态回调

### Phase 3: 冲突处理
1. 实现 `onGestureJudgeBegin` 回调（单一手势 + 组合手势）
2. 实现 `onGestureRecognizerJudgeBegin` 回调（单一手势 + 组合手势）
3. 实现 `shouldParallelWith` 机制
4. 添加手势优先级系统

### Phase 4: 高级功能
1. 添加 `preventBegin` 机制
2. 实现手势识别器动态启闭
3. 嵌套滚动支持
4. 性能优化

---

## 7. 使用示例

### 完整示例: 视频播放器手势

```rust
VideoPlayer {
    // 基础控制手势(互斥)
    gesture: ExclusiveGesture([
        // 单击: 暂停/播放
        TapGesture { 
            count: 1,
            onGestureBegan: |event| { /* 可在此显示视觉反馈 */ },
            onGestureEnded: |event| { toggle_play(); }
        },
        // 双击: 全屏切换
        TapGesture { 
            count: 2, 
            max_duration_ms: 300,
            onGestureBegan: |event| { /* 可在此显示视觉反馈 */ },
            onGestureEnded: |event| { toggle_fullscreen(); }
        },
        // 长按: 快进
        LongPressGesture { 
            duration_ms: 800,
            onGestureBegan: |event| { show_fast_forward_ui(); },
            onGestureEnded: |event| { stop_fast_forward(); }
        },
    ]),
    
    // 进度条滑动(水平)
    gesture: PanGesture {
        direction: PanDirection::Horizontal,
        min_distance: 10.0,
        onGestureBegan: |event| { show_seek_preview(); },
        onGestureChanged: |event| {
            seek_video(event.delta_x);
        },
        onGestureEnded: |event| { hide_seek_preview(); },
    },
    
    // 亮度调节(垂直滑动手势 - 只在左半屏响应)
    gesture: PanGesture {
        direction: PanDirection::Vertical,
        onGestureJudgeBegin: |gesture, context| {
            if context.location.x < context.bounds.width / 2.0 {
                GestureJudgement::Accept
            } else {
                GestureJudgement::Reject
            }
        },
        onGestureBegan: |event| { show_brightness_ui(); },
        onGestureChanged: |event| {
            adjust_brightness(event.delta_y);
        },
        onGestureEnded: |event| { hide_brightness_ui(); },
    },
    
    // 嵌套滚动处理
    ScrollView {
        gesture: PanGesture::default(),
        shouldParallelWith: |parent_gesture| {
            // 允许与父组件的 Pan 手势并行
            parent_gesture.target.is::<VideoPlayer>()
        }
    }
}
```

---

## 8. 与现有系统的兼容

### 8.1 向后兼容

```rust
// 现有 API 保持不变，内部映射到新系统
View {
    onTap: |event| { /* ... */ }         // -> TapGesture { count: 1, onGestureEnded: ... }
    onDoubleTap: |event| { /* ... */ }  // -> TapGesture { count: 2, onGestureEnded: ... }
    onLongPress: |event| { /* ... */ }  // -> LongPressGesture { onGestureEnded: ... }
    onPanUpdate: |event| { /* ... */ }  // -> PanGesture { onGestureChanged: ... }
}
```

### 8.2 渐进式采用

```rust
// 新旧 API 可以混合使用
View {
    // 简单手势使用旧 API
    onTap: |event| { show_menu(); },
    
    // 复杂手势使用新 API
    gesture: SequenceGesture([
        TapGesture { 
            count: 2,
            onGestureBegan: |event| { log.info("Double tap began"); },
            onGestureEnded: |event| { log.info("Double tap ended"); }
        },
        LongPressGesture { 
            duration_ms: 500,
            onGestureBegan: |event| { log.info("Long press began"); },
            onGestureEnded: |event| { log.info("Long press ended"); }
        }
    ])
}
```

---

## 9. 总结

这套手势编排系统的核心优势：

1. **统一模型**: 单一/组合手势统一处理，降低心智负担
2. **声明式 DSL**: 直观的 gesture 配置语法
3. **灵活编排**: 顺序/并行/互斥三种策略满足复杂交互
4. **精确控制**: 裁决机制允许应用干预识别过程
5. **可扩展**: 易于添加新手势类型和组合策略

参考了华为 HarmonyOS 的优秀设计，同时结合 Flutter GestureArena 的竞争机制和 Rust 的类型安全优势，打造一套强大且易用的手势系统。
