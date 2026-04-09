# Dyxel TextInput 组件设计规格书 (2026-04-09)

## 1. 设计目标
为 Dyxel 提供一个高性能、跨平台一致且符合响应式编程范式的文本输入组件。

*   **架构选型**: 混合事务型 (Hybrid Transaction-based)，解耦合 Native IME 与 WASM 状态。
*   **状态管理**: 基于 Signal 的响应式值 (Reactive Value) 绑定。
*   **渲染逻辑**: 完全由 WASM (Vello) 负责光标、选区及高亮渲染。
*   **焦点管理**: 具备焦点抢占 (Focus Preemption) 与同步机制。

## 2. 核心架构

### 2.1 混合事务模型 (The Hybrid Model)
借用 Flutter 的 `TextInputClient` 理念：
1.  **Host (Native)**: 仅作为输入法代理。负责展示软键盘、处理 IME (中日韩输入法) 候选词、采集按键事件。
2.  **Guest (WASM)**: 状态真相来源 (Source of Truth)。负责逻辑过滤、状态调和 (Reconciliation) 及最终布局计算。
3.  **FFI Bridge**: 通过 `TextUpdateTransaction` 进行异步同步。

### 2.2 数据模型

```rust
#[derive(Clone, Default, PartialEq)]
pub struct TextState {
    pub text: String,               // 文本内容
    pub selection: TextSelection,   // 选区（start == end 时为光标位置）
    pub composing: Option<Range>,   // IME 组合区范围
}

pub struct TextSelection {
    pub start: usize,
    pub end: usize,
    pub base_offset: usize,
    pub extent_offset: usize,
}
```

## 3. API 设计

### 3.1 响应式绑定
开发者通过 `Signal<TextState>` 进行交互，而非简单的 `String`。

```rust
// 示例用法
let state = create_signal(TextState::new("Hello"));

TextInput {
    value: state,
    on_change: move |new_state| state.set(new_state),
    placeholder: "请输入内容...",
    keyboard_type: KeyboardType::Default,
}
```

## 4. 焦点抢占机制 (Focus Management)

### 4.1 全局 FocusManager
系统维护一个全局的 `FocusManager`，确保同一时间只有一个节点持有焦点。

1.  **焦点请求 (RequestFocus)**: 当新节点被点击或程序触发焦点切换时。
2.  **焦点抢占 (Preemption)**: 
    *   通知当前焦点节点失去焦点 (Blur)。
    *   旧节点执行“最后一次事务同步”，确保数据完整。
    *   更新全局 `focused_id`。
    *   同步键盘配置 (Keyboard Options) 到 Native IME。

## 5. 渲染设计 (Vello Renderer)

### 5.1 渲染分层
1.  **底色/背景**: WASM 渲染。
2.  **选区高亮**: WASM 渲染半透明遮罩。
3.  **文本 Glyphs**: WASM 渲染。
4.  **光标 (Cursor)**: WASM 渲染。支持自定义颜色、宽度和闪烁动画。
5.  **IME 组合下划线**: WASM 渲染，用于区分已确认文本和正在输入的文本。

## 6. FFI 协议扩展 (OpCodes)

| OpCode | 名称 | 描述 |
| :--- | :--- | :--- |
| `101` | `SetTextInputFocused` | WASM 通知 Native 切换焦点并弹出/收回键盘 |
| `102` | `SyncTextState` | WASM 同步完整的 TextState 到 Native |
| `120` | `OnTextUpdate` | Native 通知 WASM 文本或选区发生了 Delta 变更 |
| `121` | `OnAction` | Native 通知 WASM 用户点击了“完成/搜索”等动作键 |

## 7. 实施路线图
1.  **Phase 1**: 建立 `TextState` 模型与基本的 `TextInput` 组件外壳。
2.  **Phase 2**: 实现 `FocusManager` 及其抢占逻辑。
3.  **Phase 3**: 完善 FFI 事务同步，支持基础文本编辑。
4.  **Phase 4**: 实现 Vello 光标与选区渲染。
5.  **Phase 5**: 高级 IME (中文输入法) 支持。
