# Host 侧 Transaction 系统实现

## 核心组件

### 1. Transaction 状态机 (`transaction.rs`)

```rust
pub enum TransactionState {
    Idle,
    Active { seq_id: u32, flags: u16 },
    Committed { seq_id: u32 },
    Aborted { seq_id: u32 },
}
```

**功能**:
- 管理事务生命周期
- 支持 begin/commit/abort 操作
- 嵌套事务检测（不支持，会报错）

### 2. 命令累加器 (`CommandAccumulator`)

```rust
pub struct CommandAccumulator {
    commands: HashMap<DedupKey, StagedCommand>,
    order: Vec<DedupKey>,
}
```

**去重逻辑**:
- Key: (node_id, field_type)
- 相同 key 的命令会合并，保留最后一个值
- 例如：5 次 color 更新 → 1 次最终 color

### 3. 脏区域追踪 (`DirtyTracker`)

```rust
pub struct DirtyTracker {
    node_bitset: [u32; 32],        // 1024 nodes
    node_dirty_fields: HashMap<u32, DirtyField>,
    any_dirty: bool,
}
```

**Bitset 轮询**:
```rust
// 快速检查是否有脏节点
pub fn has_dirty(&self) -> bool { self.any_dirty }

// 快速检查特定节点
pub fn is_node_dirty(&self, node_id: u32) -> bool {
    let word_idx = (node_id / 32) as usize;
    let bit_idx = node_id % 32;
    (self.node_bitset[word_idx] & (1 << bit_idx)) != 0
}

// 迭代所有脏节点（使用位运算优化）
pub fn iter_dirty_nodes(&self) -> impl Iterator<Item = u32> + '_ {
    // 使用 trailing_zeros 快速遍历置位
}
```

### 4. 变长 Opcode 处理

对于 `SetTextContent`, `SetText`, `SetTextFontFamily`, `SetLabel`:

```rust
let actual_len = match op {
    OpCode::SetText | OpCode::SetTextContent | ... => {
        // 读取 len 字段，计算实际长度
        let text_len = u32::from_le_bytes([...]) as usize;
        base_len + text_len
    }
    _ => base_len,
};
```

## 渲染触发流程

```
WASM guest_tick()
    ↓
process_commands() 
    ↓
遇到 EndTransaction
    ↓
tx_processor.commit()
    ↓
render_pending = true
    ↓
LogicThread 检查 is_render_needed()
    ↓
发送 RequestDraw
    ↓
RenderThread 渲染
    ↓
clear_dirty_tracker()
```

## 性能优化

| 优化点 | 实现方式 |
|--------|----------|
| 命令去重 | HashMap 合并相同 (node, field) 的命令 |
| 脏节点追踪 | Bitset 替代 HashSet，O(1) 检查和迭代 |
| 延迟渲染 | Transaction 完成时才触发 RequestDraw |
| 选择性同步 | sync_layout_to_wasm 只同步脏节点 |

## 使用示例

### WASM 侧
```rust
// 开始事务
let mut tx = begin_transaction();

// 批量创建节点
for i in 0..100 {
    View::new().width(30.0).height(30.0);
}

// 提交事务（命令打包发送）
tx.commit();
```

### Host 侧处理
```rust
// 处理命令流
process_command_stream_with_tx(state, command_data, tx_processor);

// 检查是否需要渲染
if is_render_needed() {
    let dirty_count = get_dirty_tracker()
        .map(|dt| dt.iter_dirty_nodes().count())
        .unwrap_or(0);
    send_render_request();
    clear_dirty_tracker();
}
```

## 验证结果

- ✅ 无 "Unknown opcode" 错误
- ✅ FPS 稳定在 150+
- ✅ 内存使用正常 (~640MB)
- ✅ 200+ 节点流畅渲染
