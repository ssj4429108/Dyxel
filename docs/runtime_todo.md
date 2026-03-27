Week 1: Transaction + Dirty Bitset（指令原子性）

  目标: 解决 UI 撕裂和无序风险

  Day 1-2: dyxel-shared 协议层改造
    - 添加 Transaction 结构体
    - 定义 NodeDirtyFields 位掩码
    - 修改 SharedState 支持双缓冲

  Day 3-4: WASM 侧 (sample) 改造
    - 实现 begin_transaction() / end_transaction()
    - 更新指令发送逻辑，批量打包

  Day 5: Host 侧 (dyxel-core) 改造
    - Engine 轮询 Bitset 而非逐条执行
    - 指令合并去重逻辑
    - 渲染触发改为 Transaction 完成时

  验证标准:
    - 同一帧内多次修改同一节点属性，只渲染最后一次
    - 不会出现"背景变了位置没变"的撕裂现象

  Week 2: LayoutRegistry（消除回读）

  目标: WASM 异步获取布局结果，零阻塞

  Day 1-2: 共享内存布局设计
    - 开辟 LayoutRegistry 区域 (x, y, w, h)
    - 设计节点 ID 到布局数据的索引

  Day 3-4: Host 侧自动写入
    - Taffy 布局完成后，自动同步到 LayoutRegistry
    - 脏标记避免重复写入未变更节点

  Day 5: WASM 侧读取 API
    - 暴露 get_layout(node_id) -> (x, y, w, h)
    - 示例：文本溢出检测逻辑

  验证标准:
    - WASM 可以获取任意节点布局（延迟 1 帧）
    - 文本省略号、瀑布流等场景可正常工作

  Week 3: Shadow Layout（长线优化）

  目标: WASM 侧预估布局，实现"零延迟"响应

  Day 1-2: WASM 侧集成 Taffy
    - 编译 taffy 为 WASM 模块
    - 精简配置（移除不用的 feature）

  Day 3-4: 影子布局同步机制
    - WASM 侧维护轻量布局树
    - Host/WASM 布局结果对比校验

  Day 5: 渐进式降级
    - 预估准确时零延迟
    - 预估偏差时回退到 LayoutRegistry

  验证标准:
    - 复杂布局场景下响应延迟 < 16ms

  Week 4: 分代缓冲区 + 背压（内存稳定性）

  目标: 高频更新下内存可控

  Day 1-2: 内存分区实现