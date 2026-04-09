---
name: scrum-framework-manager
description: 在项目中执行标准的 Scrum 开发流程，管理 `tasks/` 目录下的所有文档，包括 Backlog、Sprint Planning、执行跟踪和经验沉淀
---

# Skill: Scrum Framework Manager

此技能用于在项目中执行标准的 Scrum 开发流程，管理 `tasks/` 目录下的所有文档。

## 核心职责

1. **初始化架构**：如果 `tasks/` 目录不存在，按需创建目录树：
   ```
   tasks/
   ├── product_backlog.md    # 产品待办列表
   ├── lessons.md            # 经验沉淀/复盘记录
   └── sprint{N}/            # 当前 Sprint 目录
       ├── planning.md       # Sprint 计划
       ├── daily.md          # 每日站会记录
       └── review.md         # Sprint 评审与回顾
   ```

2. **Backlog 管理**：维护 `product_backlog.md`，使用以下标识状态：
   - ✅ Done - 已完成
   - 🚧 In Progress - 进行中
   - 📋 Todo - 待办
   - 💡 Idea - 想法/待定

3. **Sprint 自动化**：
   - **Planning**: 自动从 Backlog 提取 Story 并生成 `sprint{N}/planning.md`
   - **Execution**: 每次修改代码后，必须检查并更新 `planning.md` 中的任务状态
   - **Review/Retro**: 辅助生成评审报告并更新 `lessons.md`

## 强制规则

- **文档同步**：代码变更必须伴随 `tasks/` 对应文档的更新
- **验收标准**：每一个 Story 在开始前必须在 `planning.md` 中定义具体的 AC (Acceptance Criteria)
- **经验沉淀**：收到用户纠正（Corrected）时，必须立即将原因写入 `lessons.md`

## 触发场景

- 当用户说 "开始新的 Sprint" 时，触发 Planning 流程
- 当用户说 "当前进度如何" 时，分析 `planning.md` 并汇报
- 当一个任务完成时，自动执行代码审查并更新状态

## 执行流程

### 1. 初始化检查

每次技能被调用时，首先检查 `tasks/` 目录结构：

```rust
if !tasks_dir.exists() {
    create_tasks_structure();
}
```

### 2. Sprint Planning 流程

当用户触发 "开始新的 Sprint" 时：

1. 读取 `product_backlog.md` 获取所有 📋 Todo 状态的高优先级 Story
2. 计算下一个 Sprint 编号（根据现有 sprint 目录数量 + 1）
3. 创建 `sprint{N}/planning.md`，包含：
   - Sprint 目标
   - Story 列表及验收标准（AC）
   - 任务拆分及预估
   - 风险识别
4. 更新选中 Story 的状态为 🚧 In Progress

### 3. 进度跟踪流程

当用户询问 "当前进度如何" 时：

1. 读取当前 Sprint 的 `planning.md`
2. 统计各状态任务数量
3. 生成进度报告：
   - 整体完成百分比
   - 已完成任务列表
   - 进行中任务列表
   - 阻塞/风险项

### 4. 任务完成流程

当代码变更完成时：

1. 检查变更对应的 Story ID
2. 更新 `planning.md` 中对应任务状态为 ✅ Done
3. 如果 Story 下所有任务完成，更新 Story 状态为 ✅ Done
4. 触发代码审查流程

### 5. 经验沉淀流程

当用户纠正错误或提供反馈时：

1. 立即在 `lessons.md` 追加记录：
   - 日期
   - 场景描述
   - 问题/错误
   - 正确做法
   - 避免重复的建议

## 文档模板

### product_backlog.md

```markdown
# Product Backlog

## 高优先级

### Story 1: [标题]
- **状态**: 📋 Todo
- **描述**: [用户故事描述]
- **AC**: [验收标准]
- **预估**: [点数/工时]

## 中优先级

## 低优先级/想法

### Idea 1: [标题]
- **状态**: 💡 Idea
- **描述**: [想法描述]
```

### sprint{N}/planning.md

```markdown
# Sprint {N} Planning

## Sprint 目标
[一句话描述本 Sprint 目标]

## Story 列表

### Story 1: [标题]
**状态**: 🚧 In Progress
**AC**:
- [ ] AC1: [具体验收标准]
- [ ] AC2: [具体验收标准]

**任务拆分**:
- [ ] Task 1: [描述] - 预估: 2h
- [ ] Task 2: [描述] - 预估: 4h

## 风险识别
- [风险1]: [缓解措施]
```

### lessons.md

```markdown
# 经验沉淀

## 2026-04-08

### [标题]
**场景**: [什么情况下发生]
**问题**: [遇到了什么问题/错误]
**解决**: [如何解决的/正确做法]
**建议**: [如何避免重复]
```

## 与项目的集成

此技能应该与以下项目文件配合使用：
- `CLAUDE.md` - 项目全局规范
- `tasks/` 目录 - Scrum 流程文档
- 代码中的 TODO/FIXME 注释（可自动同步到 Backlog）
