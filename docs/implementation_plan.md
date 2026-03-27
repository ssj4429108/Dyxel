# Dyxel 核心优化实施计划

## 目标
解决 UI 撕裂、竞态条件和指令冗余问题，实现 Transaction 原子性和双缓冲状态。

## Phase 1: Transaction 协议 (2小时)

### 1.1 协议层扩展 (protocol.rs)
- [x] 添加 `BeginTransaction(seq_id: u32, flags: u16)`
- [x] 添加 `EndTransaction(seq_id: u32)`
- [x] 添加 `AbortTransaction(seq_id: u32)`
- [x] 添加 TransactionHeader 结构体

### 1.2 状态层扩展 (state.rs)
- [x] 添加 TransactionState 枚举
- [x] 添加 NodeDirtyFields 位掩码
- [x] 扩展 SharedState 支持 staging 缓冲区

### 1.3 Host 运行时 (runtime.rs)
- [x] 实现 Transaction 暂存逻辑
- [x] 实现 EndTransaction 时批量应用
- [x] 实现指令合并去重

### 1.4 WASM API (dyxel-view/lib.rs)
- [x] 添加 `begin_transaction() -> Transaction`
- [x] 添加 `end_transaction(tx: Transaction)`
- [x] 保持向后兼容（隐式 Transaction）

## Phase 2: 双缓冲状态 (2小时)

### 2.1 状态缓冲区设计
- [ ] StateBuffer 结构体（独立所有权）
- [ ] FrontBuffer（RenderThread 只读）
- [ ] BackBuffer（LogicThread 写入）
- [ ] StagingBuffer（Transaction 暂存）

### 2.2 Swap 机制
- [ ] EndTransaction 时 Back -> Staging
- [ ] Render 前 Staging -> Front
- [ ] 使用 Arc<Mutex<>> 保证线程安全

## Phase 3: 指令合并 (1小时)

### 3.1 Dirty Bitset 扩展
- [ ] Node 添加 `dirty_fields: u8`
- [ ] 定义 DirtyField 位枚举（Position/Size/Style/Text）

### 3.2 合并逻辑
- [ ] Transaction 内同一节点多次修改只保留最后一次
- [ ] 不同字段独立追踪（位置变化不合并到颜色）

## Phase 4: 自验证测试 (2小时)

### 4.1 单元测试
- [ ] Transaction 原子性测试
- [ ] 指令合并正确性测试
- [ ] 双缓冲 Swap 测试

### 4.2 集成测试
- [ ] 1000 节点连续更新测试
- [ ] 高频动画场景测试
- [ ] 内存稳定性测试

### 4.3 性能基准
- [ ] 对比优化前后 FPS
- [ ] 对比内存占用
- [ ] 验证无 UI 撕裂

## 验证标准

| 测试项 | 通过标准 | 验证方式 |
|--------|---------|----------|
| Transaction 原子性 | 同一帧内多次修改只渲染一次 | 日志计数 |
| 指令合并 | 冗余指令减少 >50% | 指令计数器 |
| 双缓冲 | 无竞态条件崩溃 | 压力测试 10min |
| 性能 | FPS 不下降 >5% | 性能监控 |
| 内存 | 无内存泄漏 | 趋势 0.0MB/min |

## 实施记录

### 2024-03-27
- [x] 完成协议层设计
- [x] 完成协议层实现
- [ ] 进行中: Host 运行时实现
