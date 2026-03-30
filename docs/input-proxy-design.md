# Dyxel Input Proxy 设计文档

## 概述

Input Proxy（输入代理）是 Dyxel 双轨架构中的关键组件，负责解决 WASM 与宿主环境（Host）之间的高频输入交互瓶颈。通过在宿主侧完成输入事件的规整化、命中检测和手势预处理，显著降低 JNI/WASM 调用开销，提升输入响应性能。

## 设计目标

| 目标 | 指标 |
|------|------|
| 降低输入延迟 | < 16ms（单帧内处理） |
| 减少 WASM 调用 | 批量处理，每帧最多 1 次 WASM 调用 |
| 支持高频输入 | 120Hz 采样率不丢事件 |
| 手势识别精度 | Tap 误识别率 < 1% |

## 系统架构

```
┌─────────────────────────────────────────────────────────────────┐
│                         宿主环境 (Host)                          │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────────────┐  │
│  │  Android/iOS │ → │ InputProxy  │ → │  Shared InputBuffer │  │
│  │  原始输入事件 │    │ 规整化/命中  │    │   (环形缓冲区)       │  │
│  └─────────────┘    └─────────────┘    └─────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────────┐
│                      WASM 逻辑层 (Guest)                         │
│  ┌─────────────────┐    ┌─────────────────┐    ┌─────────────┐  │
│  │ GestureRecognizer│ → │ EventDispatcher │ → │ 组件回调     │  │
│  │   (手势状态机)   │    │   (冒泡/消费)    │    │ onTap/onPan │  │
│  └─────────────────┘    └─────────────────┘    └─────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

## 第一阶段：共享输入协议

### 核心数据结构

#### 1.1 输入事件类型

```rust
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEventType {
    PointerDown = 0,
    PointerMove = 1,
    PointerUp = 2,
    PointerCancel = 3,
    MouseWheel = 4,
    KeyDown = 5,
    KeyUp = 6,
}
```

#### 1.2 原始输入事件

```rust
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RawInputEvent {
    /// 微秒级时间戳
    pub timestamp: u64,
    /// 事件类型
    pub event_type: InputEventType,
    /// 多点触控 ID（单指为 0）
    pub pointer_id: u32,
    /// 世界坐标 X
    pub x: f32,
    /// 世界坐标 Y
    pub y: f32,
    /// 按压力度（0.0 ~ 1.0）
    pub pressure: f32,
    /// X 方向增量（用于滚动）
    pub delta_x: f32,
    /// Y 方向增量（用于滚动）
    pub delta_y: f32,
    /// 宿主侧预计算的命中节点 ID
    pub target_node_id: u32,
    /// 扩展标志位
    pub flags: u32,
}
```

#### 1.3 环形缓冲区

```rust
/// 缓冲区容量：约 100 个事件（4KB）
pub const INPUT_BUFFER_CAPACITY: usize = 100;

#[repr(C)]
pub struct InputBuffer {
    /// 写入位置（宿主侧）
    pub write_idx: u32,
    /// 读取位置（WASM 侧）
    pub read_idx: u32,
    /// 溢出计数（调试使用）
    pub overflow_count: u32,
    /// 事件存储数组
    pub events: [RawInputEvent; INPUT_BUFFER_CAPACITY],
}
```

### SharedBuffer 扩展

在现有 `SharedBuffer` 中添加输入缓冲区：

```rust
#[repr(C, align(16))]
pub struct SharedBuffer {
    // 现有字段...
    pub command_len: u32,
    pub max_node_id: u32,
    pub _padding: [u32; 2],
    pub command_data: [u8; MAX_COMMAND_BYTES],
    pub layout_results: [LayoutResult; MAX_NODES],
    pub dirty_mask: [u32; 32],
    // 新增：输入事件环形缓冲区
    pub input_buffer: InputBuffer,
}
```

## 第二阶段：宿主侧输入代理

### 2.1 InputProxy 结构

```rust
pub struct InputProxy {
    /// 屏幕到世界坐标的变换矩阵
    screen_to_world: Affine,
    /// 热区扩展值（dp）
    hit_area_expansion: f32,
    /// 多点触控状态跟踪
    pointer_states: HashMap<u32, PointerState>,
}

pub struct PointerState {
    pub start_x: f32,
    pub start_y: f32,
    pub start_time: u64,
    pub is_panning: bool,
}
```

### 2.2 核心处理流程

```
原生输入事件
    ↓
[1] 坐标投影（考虑 DPI/缩放）
    ↓
[2] 热区扩展命中检测
    ↓
[3] 创建 RawInputEvent
    ↓
[4] 压入环形缓冲区
    ↓
等待 WASM 下一帧消费
```

### 2.3 命中检测（带热区扩展）

```rust
impl InputProxy {
    /// 热区扩展命中检测
    fn hit_test_with_expansion(
        &self,
        point: Vec2,
        state: &SharedState,
        expansion: f32,
    ) -> Option<u32> {
        // 最小点击目标：44dp
        const MIN_TOUCH_TARGET: f32 = 44.0;
        
        // 深度优先遍历（子节点优先）
        self.hit_test_recursive(id, point, state, Vec2::ZERO, expansion)
    }
    
    fn hit_test_recursive(
        &self,
        id: u32,
        point: Vec2,
        state: &SharedState,
        parent_pos: Vec2,
        expansion: f32,
    ) -> Option<u32> {
        let node = state.nodes.get(&id)?;
        let layout = state.taffy.layout(node.taffy_node).ok()?;
        let global_pos = parent_pos + Vec2::new(layout.location.x as f64, layout.location.y as f64);
        
        // 计算带热区扩展的命中矩形
        let width = layout.size.width.max(MIN_TOUCH_TARGET) + expansion * 2.0;
        let height = layout.size.height.max(MIN_TOUCH_TARGET) + expansion * 2.0;
        let rect = KRect::from_origin_size(
            (global_pos.x - expansion, global_pos.y - expansion),
            (width as f64, height as f64),
        );
        
        // 优先检查子节点（从顶层到底层）
        for &child_id in node.children.iter().rev() {
            if let Some(hit) = self.hit_test_recursive(child_id, point, state, global_pos, expansion) {
                return Some(hit);
            }
        }
        
        // 检查当前节点
        if rect.contains(Point::new(point.x, point.y)) {
            if state.click_listeners.contains(&id) || has_gesture_handler(id) {
                return Some(id);
            }
        }
        
        None
    }
}
```

### 2.4 Android 集成

```kotlin
// MainActivity.kt 触摸事件处理
sv.setOnTouchListener { v, event ->
    if (!isInitialized) return@setOnTouchListener false
    
    when (event.actionMasked) {
        MotionEvent.ACTION_DOWN -> {
            engine.host.onPointerDown(
                event.getPointerId(0),
                event.x, 
                event.y,
                event.getPressure(0)
            )
        }
        MotionEvent.ACTION_POINTER_DOWN -> {
            val idx = event.actionIndex
            engine.host.onPointerDown(
                event.getPointerId(idx),
                event.getX(idx),
                event.getY(idx),
                event.getPressure(idx)
            )
        }
        MotionEvent.ACTION_MOVE -> {
            for (i in 0 until event.pointerCount) {
                engine.host.onPointerMove(
                    event.getPointerId(i),
                    event.getX(i),
                    event.getY(i)
                )
            }
        }
        MotionEvent.ACTION_UP -> {
            engine.host.onPointerUp(
                event.getPointerId(0),
                event.x,
                event.y
            )
        }
        MotionEvent.ACTION_POINTER_UP -> {
            val idx = event.actionIndex
            engine.host.onPointerUp(
                event.getPointerId(idx),
                event.getX(idx),
                event.getY(idx)
            )
        }
        MotionEvent.ACTION_CANCEL -> {
            engine.host.onPointerCancel()
        }
    }
    v.performClick()
    true
}
```

## 第三阶段：WASM 侧手势合成器

### 3.1 手势类型定义

```rust
pub enum Gesture {
    /// 点击
    Tap { node_id: u32, x: f32, y: f32 },
    /// 长按
    LongPress { node_id: u32, x: f32, y: f32 },
    /// 平移开始
    PanStart { node_id: u32, x: f32, y: f32 },
    /// 平移更新
    PanUpdate { 
        node_id: u32, 
        x: f32, 
        y: f32, 
        delta_x: f32, 
        delta_y: f32 
    },
    /// 平移结束
    PanEnd { node_id: u32, x: f32, y: f32 },
}
```

### 3.2 手势识别状态机

```rust
enum PointerState {
    Idle,
    Down { 
        node_id: u32, 
        start_x: f32, 
        start_y: f32, 
        down_time: u64 
    },
    Panning {
        node_id: u32,
        last_x: f32,
        last_y: f32,
    },
}

pub struct GestureRecognizer {
    state: PointerState,
    config: GestureConfig,
}

pub struct GestureConfig {
    /// Tap 超时（300ms）
    pub tap_timeout_ms: u64,
    /// 触摸偏差阈值（10px）
    pub tap_slop: f32,
    /// 长按超时（500ms）
    pub long_press_timeout_ms: u64,
}
```

### 3.3 状态转换图

```
                    PointerDown
                         ↓
    ┌────────────────────┼────────────────────┐
    ↓                    ↓                    ↓
 PointerUp          PointerMove         超时 (>300ms)
(时间<300ms)      (位移>10px)                ↓
    ↓                    ↓              LongPress
   Tap                   ↓
                   PanStart
                      ↓ ↓
              PointerMove PointerUp
                  ↓         ↓
            PanUpdate    PanEnd
```

### 3.4 帧内事件合并

```rust
/// 合并高频事件，减少处理器负担
fn coalesce_events(events: Vec<RawInputEvent>) -> Vec<RawInputEvent> {
    let mut result = Vec::new();
    let mut i = 0;
    
    while i < events.len() {
        let current = &events[i];
        
        // 检查是否可以和后续事件合并
        if current.event_type == InputEventType::PointerMove {
            let mut last_idx = i;
            let mut accumulated_delta_x = current.delta_x;
            let mut accumulated_delta_y = current.delta_y;
            
            for j in (i + 1)..events.len() {
                let next = &events[j];
                if next.event_type == InputEventType::PointerMove
                    && next.target_node_id == current.target_node_id {
                    last_idx = j;
                    accumulated_delta_x += next.delta_x;
                    accumulated_delta_y += next.delta_y;
                } else {
                    break;
                }
            }
            
            if last_idx > i {
                // 创建合并后的事件
                let mut merged = *current;
                merged.delta_x = accumulated_delta_x;
                merged.delta_y = accumulated_delta_y;
                merged.x = events[last_idx].x;
                merged.y = events[last_idx].y;
                result.push(merged);
                i = last_idx + 1;
                continue;
            }
        }
        
        result.push(*current);
        i += 1;
    }
    
    result
}
```

### 3.5 事件冒泡机制

```rust
pub enum EventResponse {
    /// 消费事件，停止冒泡
    Consume,
    /// 继续冒泡到父节点
    Bubble,
}

pub fn dispatch_with_bubble(
    node_id: u32, 
    event_type: &str, 
    gesture: &Gesture,
    handlers: &HashMap<u32, EventHandlerSet>,
) -> EventResponse {
    let mut current_id = Some(node_id);
    
    while let Some(id) = current_id {
        if let Some(handler_set) = handlers.get(&id) {
            let response = match event_type {
                "onTap" => handler_set.on_tap.as_ref().map(|h| h(gesture)),
                "onPanStart" => handler_set.on_pan_start.as_ref().map(|h| h(gesture)),
                "onPanUpdate" => handler_set.on_pan_update.as_ref().map(|h| h(gesture)),
                "onPanEnd" => handler_set.on_pan_end.as_ref().map(|h| h(gesture)),
                _ => None,
            };
            
            if response == Some(EventResponse::Consume) {
                return EventResponse::Consume;
            }
        }
        
        // 继续冒泡到父节点
        current_id = get_parent_id(id);
    }
    
    EventResponse::Bubble
}
```

## 性能优化策略

### 1. 帧内合并（Event Coalescing）

| 事件类型 | 合并策略 | 效果 |
|---------|---------|------|
| PointerMove | 同目标节点的连续事件合并 | 120Hz → 60fps 不丢精度 |
| MouseWheel | delta_x/delta_y 累加 | 减少滚动事件数量 |

### 2. 热区扩展（Hit-area Expansion）

```rust
/// 移动端最小点击目标 44dp
const MIN_TOUCH_TARGET: f32 = 44.0;
/// 热区扩展值 8dp
const HIT_EXPANSION: f32 = 8.0;
```

对于小于 44dp 的节点，自动扩展热区提高点击率。

### 3. 批量处理

```rust
// 每帧开始时批量处理所有累积事件
fn process_input_events() {
    let events: Vec<RawInputEvent> = unsafe {
        SHARED_BUFFER.input_buffer.drain().collect()
    };
    
    // 先合并再处理
    let merged = coalesce_events(events);
    
    for event in merged {
        process_single_event(event);
    }
}
```

## 实施路线图

### Week 1: 共享输入协议

| 天数 | 任务 | 输出 |
|------|------|------|
| 1-2 | 定义 RawInputEvent 和 InputBuffer | `dyxel-shared/src/input.rs` |
| 3-4 | 修改 SharedBuffer 添加输入缓冲区 | 更新后的 protocol.rs |
| 5 | 缓冲区读写单元测试 | 测试用例通过率 100% |

### Week 2: 宿主侧代理

| 天数 | 任务 | 输出 |
|------|------|------|
| 1-2 | 实现 InputProxy 结构和坐标投影 | `dyxel-core/src/input_proxy.rs` |
| 3-4 | 热区扩展命中检测 | hit_test_with_expansion 实现 |
| 5 | Android 多指事件支持 | 更新 MainActivity.kt |

### Week 3: WASM 手势合成器

| 天数 | 任务 | 输出 |
|------|------|------|
| 1-2 | 实现 GestureRecognizer 状态机 | `dyxel-view/src/gesture.rs` |
| 3 | 帧内合并算法 | coalesce_events 实现 |
| 4 | 事件冒泡机制 | dispatch_with_bubble 实现 |
| 5 | 集成到 dyxel_view_tick | 更新 lib.rs |

### Week 4: 测试与优化

| 天数 | 任务 | 输出 |
|------|------|------|
| 1 | 性能监控（事件处理耗时） | 性能日志输出 |
| 2-3 | 手势测试用例 | Tap/Pan 测试通过 |
| 4 | 边界情况处理 | 缓冲区溢出、快速滑动测试 |
| 5 | 文档和代码审查 | 设计文档更新 |

## 风险评估与缓解措施

| 风险 | 影响 | 可能性 | 缓解措施 |
|------|------|--------|---------|
| **环形缓冲区溢出** | 事件丢失 | 中 | 添加溢出计数器；生产环境使用双缓冲；WARN 日志监控 |
| **手势误识别** | 用户体验差 | 中 | 可配置阈值；调试模式可视化触摸区域；A/B 测试验证 |
| **坐标投影误差** | 点击偏移 | 低 | 渲染和输入共享变换矩阵；校准测试工具；自动化截图对比测试 |
| **多指事件顺序错乱** | 手势异常 | 低 | 严格按 pointer_id 维护状态机；参考 Android GestureDetector 源码 |
| **性能回退** | 帧率下降 | 低 | 持续监控输入处理耗时；保持命中检测 < 0.1ms 目标 |

## 接口变更清单

### dyxel-shared（新增）

- `struct RawInputEvent`
- `struct InputBuffer`
- `enum InputEventType`

### dyxel-core（修改）

- `SharedBuffer.input_buffer: InputBuffer`（新增字段）
- `struct InputProxy`（新增）
- `fn on_pointer_down/move/up/cancel`（替换 on_touch）

### dyxel-view（新增）

- `struct GestureRecognizer`
- `enum Gesture`
- `fn process_input_events()`

### Android（修改）

- `onTouch` → `onPointerDown/Move/Up/Cancel`

## 测试策略

### 单元测试

1. **缓冲区测试**: 写入/读取/溢出边界
2. **命中检测**: 单节点、嵌套节点、热区扩展
3. **手势识别**: Tap、Pan 状态机转换
4. **事件合并**: 连续 Move、Wheel 累加

### 集成测试

1. **端到端**: Android 触摸 → WASM 回调
2. **性能**: 120Hz 采样不丢事件
3. **压力**: 快速连续点击、多点触控

### 手动测试

1. **命中精度**: 小按钮（< 20dp）点击测试
2. **手势体验**: Tap vs Pan 区分度
3. **边界情况**: 手指滑出屏幕、快速切换应用

## 参考资料

- [Android Input 系统架构](https://source.android.com/docs/core/interaction/input)
- [Flutter GestureArena 设计](https://docs.flutter.dev/ui/interactions/gestures)
- [Web Pointer Events 规范](https://www.w3.org/TR/pointerevents/)

---

*文档版本: 1.0*
*最后更新: 2026-03-30*
*作者: Dyxel Team*
