# Transaction API 使用文档

## 概述

Transaction API 允许将多个 UI 操作批量打包成一个原子事务，减少 Host-WASM 通信开销，提高渲染性能。

## 核心 API

### 1. 基础事务

```rust
use dyxel_view::{begin_transaction, Transaction};

// 开始一个事务
let mut tx = begin_transaction();

// 执行多个 UI 操作
let child1 = View::new().width(100.0).height(100.0);
let child2 = View::new().width(100.0).height(100.0);
View { id: 0 }.child(child1.id).child(child2.id);

// 提交事务（所有命令一次性发送）
tx.commit();
```

### 2. 作用域事务（推荐）

```rust
use dyxel_view::with_transaction;

// 使用闭包，自动提交
with_transaction(|tx| {
    for i in 0..100 {
        let child = View::new()
            .width(30.0)
            .height(30.0)
            .color((200, 50, 50));
        View { id: 0 }.child(child.id);
    }
});
// 事务在此处自动提交
```

### 3. 带标志的事务

```rust
use dyxel_view::{begin_transaction_with_flags, TransactionFlags};

// 使用特定标志开始事务
let mut tx = begin_transaction_with_flags(TransactionFlags::Mergeable as u16);
// ... 操作 ...
tx.commit();
```

### 4. 事务中止

```rust
let mut tx = begin_transaction();

// 执行一些操作...
let child = View::new().width(100.0);

// 如果需要撤销所有操作
tx.abort();  // 所有命令被丢弃
```

## 事务生命周期

```
┌─────────────────────────────────────────┐
│           begin_transaction()           │
│              BeginTransaction           │
│                   cmd                   │
└─────────────────┬───────────────────────┘
                  │
                  ▼
┌─────────────────────────────────────────┐
│           UI Operations                 │
│  CreateNode, SetColor, AddChild, ...    │
│  (commands buffered in shared buffer)   │
└─────────────────┬───────────────────────┘
                  │
        ┌─────────┴─────────┐
        ▼                   ▼
┌───────────────┐   ┌───────────────┐
│    commit()   │   │    abort()    │
│  EndTransaction │   │ AbortTransaction│
└───────────────┘   └───────────────┘
```

## 性能建议

1. **批量创建节点**：使用事务批量创建多个节点
   ```rust
   with_transaction(|_| {
       for i in 0..100 {
           View::new().width(30.0).height(30.0);
       }
   });
   ```

2. **批量更新属性**：动画更新使用事务打包
   ```rust
   with_transaction(|_| {
       for i in 0..50 {
           View { id: i }.color((r, g, b));
       }
   });
   ```

3. **避免嵌套事务**：当前实现不支持嵌套事务

4. **及时提交**：事务持有期间会累积命令，及时提交避免 buffer 溢出

## 实现细节

- 事务使用 `seq_id` 跟踪，支持多事务并发（未来扩展）
- `Drop`  trait 确保未提交事务自动中止
- 命令直接写入 `SHARED_BUFFER`，commit 时无需复制
