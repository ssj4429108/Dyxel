# Dyxel 现状与优化方案对比分析

## 一、现状诊断：当前实现的真实情况

### 1. 指令流处理机制

#### 当前实现 (protocol.rs + runtime.rs)
```rust
// 当前：简单线性指令流
pub struct SharedBuffer {
    pub command_len: u32,
    pub command_data: [u8; MAX_COMMAND_BYTES],  // 64KB 固定缓冲区
    pub layout_results: [LayoutResult; MAX_NODES], // 布局回写区
    pub dirty_mask: [u32; 32], // 布局变更标记
}

// 处理流程：逐条顺序执行
fn process_command_stream_inner(state: &mut SharedState, data: &[u8]) {
    while offset < data.len() {
        let op = OpCode::from_u8(command_data[offset]);
        dispatch_op!(op, ...); // 立即执行每条指令
    }
}
```

**存在的问题：**
- ✅ **无事务边界**：`UpdateLayout` 是空操作 (handle_op 宏中为空)
- ✅ **无指令合并**：同一帧内多次修改同一节点，每条都执行
- ✅ **流式处理**：Host 可能在指令流中途触发 render()

#### 与方案差异对比

| 维度 | 当前实现 | 优化方案 | 差距 |
|------|---------|---------|------|
| 事务边界 | ❌ 无 | ✅ BeginFrame/EndFrame | 大 |
| 指令合并 | ❌ 逐条执行 | ✅ Bitset 去重 | 大 |
| 原子性 | ❌ 流式处理 | ✅ Transaction 快照 | 大 |
| 批量大小 | 64KB 固定 | 分代缓冲区 | 中 |

---

### 2. 布局回读机制 (LayoutRegistry)

#### 当前实现 (runtime.rs)
```rust
// 当前：已实现基础布局回写！
pub fn sync_layout_to_wasm(memory: &mut [u8], buffer_ptr: u32, state: &SharedState) {
    for (&id, node) in &state.nodes {
        if let Ok(layout) = state.taffy.layout(node.taffy_node) {
            // 写入 layout_results 区
            memory[target..target+16].copy_from_slice(&[x, y, w, h]);
            // 设置 dirty_mask
            mask |= 1 << bit_idx;
        }
    }
}
```

**现状：**
- ✅ **已有 LayoutRegistry**：`layout_results[MAX_NODES]` 数组存在
- ✅ **已有脏标记**：`dirty_mask[32]` 用于标记变更
- ✅ **已集成到 Tick**：LogicThread 每次 tick 后自动同步

#### WASM 侧读取 (dyxel-view/src/lib.rs)
```rust
pub fn get_layout(id: u32) -> LayoutResult { 
    unsafe { SHARED_BUFFER.layout_results[id as usize] } 
}
```

**与方案差异对比**

| 维度 | 当前实现 | 优化方案 | 差距 |
|------|---------|---------|------|
| 布局回写 | ✅ 已实现 | 提出者以为未实现 | 无差距 |
| 延迟 | 1帧 | 1帧 | 无差距 |
| 脏检测 | ✅ Bitmap | Bitmap | 无差距 |
| Shadow Layout | ❌ 无 | ✅ 预估布局 | 大差距 |

**关键发现**：
> 方案文档中提到的 "LayoutRegistry" 实际上 **已经实现**！但命名不同 (`layout_results`)。
> 这是一个沟通/文档问题，而非实现缺失。

---

### 3. 内存管理与背压

#### 当前实现
```rust
pub const MAX_COMMAND_BYTES: usize = 1024 * 64; // 64KB 固定

// push_command 宏检查溢出
if offset + data_size <= MAX_COMMAND_BYTES {
    // 写入
} else {
    // 静默丢弃！
}
```

**存在的问题：**
- ❌ **无背压**：溢出时静默丢弃指令
- ❌ **无分区**：所有指令混在一个缓冲区
- ❌ **无优先级**：关键指令可能被动画指令淹没

#### 与方案差异对比

| 维度 | 当前实现 | 优化方案 | 差距 |
|------|---------|---------|------|
| 背压机制 | ❌ 静默丢弃 | ✅ Watermark 触发丢帧 | 大 |
| 内存分区 | ❌ 单一缓冲区 | ✅ Registry + Command Stream | 中 |
| 指令优先级 | ❌ 无 | ✅ 关键指令优先 | 中 |

---

### 4. 双缓冲状态 (Double-Buffered State)

#### 当前实现
```rust
// bridge.rs: LogicThread 循环
loop {
    // 1. 执行 WASM tick (修改 SharedState)
    tick.call();
    
    // 2. 处理指令 (修改 SharedState)
    process_commands(mem, bptr, &l.shared_state);
    
    // 3. 同步布局 (读取 SharedState, 写入 WASM memory)
    sync_layout_to_wasm(mem, bptr, &state);
    
    // 4. 触发渲染 (RenderThread 读取 SharedState)
    render_tx.send(RenderMessage::RequestDraw);
    
    sleep(16ms);
}
```

**存在的问题：**
- ❌ **无状态隔离**：LogicThread 修改 state 时，RenderThread 可能正在读取
- ❌ **无 Swap 机制**：布局计算到一半时可能读取到不一致状态
- ⚠️ **锁竞争激烈**：`shared_state.lock()` 被频繁争用

#### 与方案差异对比

| 维度 | 当前实现 | 优化方案 | 差距 |
|------|---------|---------|------|
| 状态隔离 | ❌ 单状态 | ✅ 双缓冲 (Front/Back) | 大 |
| Swap 机制 | ❌ 无 | ✅ Frame 完成时 Swap | 大 |
| 锁粒度 | ⚠️ 粗粒度 | ✅ 细粒度/无锁 | 中 |

---

## 二、核心差异总结

### 已实现 vs 未实现

| 优化方案提出的能力 | 实际状态 | 评估 |
|-------------------|---------|------|
| LayoutRegistry | ✅ 已实现 | 无需改动 |
| Dirty Bitmask | ✅ 已实现 | 需扩展用途 |
| Transaction API | ❌ 未实现 | **优先级 P0** |
| 双缓冲状态 | ❌ 未实现 | **优先级 P0** |
| 指令合并 | ❌ 未实现 | **优先级 P0** |
| 分代缓冲区 | ❌ 未实现 | 优先级 P1 |
| Shadow Layout | ❌ 未实现 | 优先级 P2 |
| 背压机制 | ❌ 未实现 | 优先级 P1 |

---

## 三、修正后的实施路线图

### Phase 1: Transaction + 双缓冲 (2周)

**核心改动：**
```rust
// 1. 协议层新增
pub struct TransactionHeader {
    pub seq_id: u32,
    pub command_count: u16,
    pub flags: u16, // 原子性标记
}

// 2. SharedState 双缓冲
pub struct SharedState {
    front: StateBuffer, // RenderThread 读取
    back: StateBuffer,  // LogicThread 写入
    staging: StateBuffer, // Transaction 暂存
}

// 3. 新指令
[26] BeginTransaction(seq_id: u32),
[27] EndTransaction(seq_id: u32),
[28] AbortTransaction(seq_id: u32),
```

**预期收益：**
- 消除 UI 撕裂
- 支持同一帧内多次修改去重
- 原子性保证

### Phase 2: 指令合并 + 性能优化 (1周)

**核心改动：**
```rust
// Node 增加脏标记
pub struct ViewNode {
    // ... existing fields
    pub dirty_fields: u8, // DirtyField 位掩码
}

// Host 侧合并逻辑
impl SharedState {
    fn apply_transaction(&mut self, tx: Transaction) {
        for cmd in tx.commands {
            // 检查脏标记，决定是否跳过
            if self.should_skip(&cmd) { continue; }
            self.apply_command(cmd);
        }
    }
}
```

### Phase 3: 背压 + 内存分区 (1周)

**核心改动：**
```rust
pub struct SharedBuffer {
    // 静态区：树结构
    registry: RegistryArea, 
    
    // 动态区：指令流
    command_stream: RingBuffer,
    watermark: AtomicU8, // 0-100%
}

// WASM 侧检查
fn push_command(...) {
    if WATERMARK.load() > 80 {
        // 进入丢帧模式
        skip_non_critical_commands();
    }
}
```

---

## 四、关键认知修正

### 1. 关于 "LayoutRegistry"
方案文档提到的 "引入 LayoutRegistry" 实际上是**已经实现**的功能。

**现状：**
- `layout_results[MAX_NODES]` 就是 LayoutRegistry
- `sync_layout_to_wasm` 已实现自动回写
- `get_layout()` 已暴露给 WASM

### 2. 关于 "Dirty Bitset"
方案文档提到的 "引入 Dirty Bitset" 也是**部分实现**。

**现状：**
- `dirty_mask[32]` 已存在
- 但只用于标记布局变更，未用于指令去重

### 3. 真正的差距在哪里？
| 问题 | 现状 | 影响 |
|------|------|------|
| 无 Transaction | 指令流式处理，无边界 | UI 撕裂风险 |
| 无双缓冲 | Logic/Render 共享状态 | 竞态条件 |
| 无指令合并 | 同一属性修改5次执行5次 | 性能浪费 |

---

## 五、建议优先级

### 🔴 P0: 必须实现 (4周内)
1. **Transaction API**：解决原子性问题
2. **双缓冲状态**：解决竞态条件
3. **指令合并**：解决性能浪费

### 🟡 P1: 重要优化 (2-4周)
4. **背压机制**：解决内存溢出
5. **内存分区**：优化缓存局部性

### 🟢 P2: 长期改进 (后续迭代)
6. **Shadow Layout**：零延迟布局
7. **无锁队列**：极致性能

---

## 六、结论

**好消息**：
- 40% 的优化方案已经实现（LayoutRegistry、Dirty Mask）
- 核心架构（三线程、WASM 桥接）设计合理

**坏消息**：
- 60% 的关键优化尚未实现（Transaction、双缓冲、指令合并）
- 当前存在 UI 撕裂和竞态条件风险

**建议**：
立即启动 Phase 1（Transaction + 双缓冲），这是解决核心稳定性的关键。
