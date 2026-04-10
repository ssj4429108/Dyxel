// crates/dyxel-view/src/focus_manager.rs
//! 通用焦点管理器
//!
//! 支持所有 View 组件的焦点管理，包括：
//! - 多种焦点能力（Keyboard, Activatable, Selectable）
//! - 焦点栈（支持嵌套焦点）
//! - 焦点记忆和恢复

use std::cell::RefCell;
use std::collections::HashMap;

/// 焦点能力类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FocusCapability {
    /// 可接收键盘输入（TextInput）
    Keyboard,
    /// 可激活（Button、Checkbox）
    Activatable,
    /// 可选择（ListItem、Radio）
    Selectable,
}

impl FocusCapability {
    /// 默认是否显示焦点指示器
    pub fn shows_indicator(&self) -> bool {
        match self {
            FocusCapability::Keyboard => true,
            FocusCapability::Activatable => true,
            FocusCapability::Selectable => true,
        }
    }

    /// 是否自动接收焦点（点击后自动聚焦）
    pub fn auto_focus_on_tap(&self) -> bool {
        match self {
            FocusCapability::Keyboard => true,
            FocusCapability::Activatable => false,
            FocusCapability::Selectable => true,
        }
    }
}

/// 焦点节点信息
#[derive(Debug, Clone)]
pub struct FocusNode {
    pub id: u32,
    pub capabilities: Vec<FocusCapability>,
    /// 当前激活的能力
    pub active_capability: FocusCapability,
}

/// 焦点事件
#[derive(Debug, Clone)]
pub enum FocusEvent {
    /// 获得焦点
    FocusGained {
        node_id: u32,
        capability: FocusCapability,
    },
    /// 失去焦点
    FocusLost {
        node_id: u32,
        capability: FocusCapability,
    },
    /// 焦点能力切换（同节点不同能力）
    CapabilityChanged {
        node_id: u32,
        old: FocusCapability,
        new: FocusCapability,
    },
}

/// 焦点监听器类型
pub type FocusListener = Box<dyn Fn(&FocusEvent)>;

/// 通用焦点管理器
///
/// 设计为 thread_local 单例，在 WASM 单线程环境中使用
pub struct FocusManager {
    /// 当前焦点栈（支持嵌套焦点）
    focus_stack: Vec<FocusNode>,
    /// 所有可聚焦节点
    focusables: HashMap<u32, FocusNode>,
    /// 焦点事件监听器
    listeners: Vec<FocusListener>,
    /// 焦点历史（用于焦点记忆）
    focus_history: Vec<u32>,
    /// 最大历史长度
    max_history: usize,
}

impl FocusManager {
    /// 创建新的焦点管理器
    pub fn new() -> Self {
        Self {
            focus_stack: Vec::new(),
            focusables: HashMap::new(),
            listeners: Vec::new(),
            focus_history: Vec::new(),
            max_history: 10,
        }
    }

    /// 注册可聚焦节点
    pub fn register(&mut self, node_id: u32, capabilities: Vec<FocusCapability>) {
        if capabilities.is_empty() {
            return;
        }

        let active = capabilities[0];
        self.focusables.insert(
            node_id,
            FocusNode {
                id: node_id,
                capabilities,
                active_capability: active,
            },
        );
    }

    /// 注销节点
    pub fn unregister(&mut self, node_id: u32) {
        // 如果当前聚焦的是该节点，先清除焦点
        if self.is_focused(node_id) {
            self.clear_focus();
        }
        self.focusables.remove(&node_id);
    }

    /// 请求焦点
    ///
    /// 返回 true 表示焦点发生变化，false 表示无变化（已是焦点）
    pub fn request_focus(&mut self, node_id: u32, capability: FocusCapability) -> bool {
        // 检查节点是否支持该能力
        let node = match self.focusables.get(&node_id) {
            Some(n) if n.capabilities.contains(&capability) => n.clone(),
            _ => return false,
        };

        // 检查是否已经是该能力的焦点
        if let Some(current) = self.focus_stack.last() {
            if current.id == node_id && current.active_capability == capability {
                return false;
            }

            // 通知旧焦点失去焦点
            self.notify(&FocusEvent::FocusLost {
                node_id: current.id,
                capability: current.active_capability,
            });
        }

        // 创建新的焦点节点
        let new_node = FocusNode {
            id: node_id,
            capabilities: node.capabilities,
            active_capability: capability,
        };

        // 压入焦点栈
        self.focus_stack.push(new_node.clone());

        // 记录历史
        self.add_to_history(node_id);

        // 通知新焦点获得焦点
        self.notify(&FocusEvent::FocusGained {
            node_id,
            capability,
        });

        true
    }

    /// 简化的焦点请求（使用默认能力）
    pub fn request_focus_simple(&mut self, node_id: u32) -> bool {
        if let Some(node) = self.focusables.get(&node_id) {
            let cap = node.capabilities[0];
            self.request_focus(node_id, cap)
        } else {
            false
        }
    }

    /// 切换到同一节点的不同能力
    pub fn switch_capability(&mut self, capability: FocusCapability) -> bool {
        // Check if we have a focused node
        if self.focus_stack.is_empty() {
            return false;
        }

        // Get the current node's data
        let current = self.focus_stack.last().unwrap();
        if current.active_capability == capability {
            return false;
        }
        if !current.capabilities.contains(&capability) {
            return false;
        }

        let node_id = current.id;
        let old = current.active_capability;

        // Now do the mutable operation
        if let Some(current) = self.focus_stack.last_mut() {
            current.active_capability = capability;
        }

        self.notify(&FocusEvent::CapabilityChanged {
            node_id,
            old,
            new: capability,
        });

        true
    }

    /// 清除焦点（点击空白区域）
    pub fn clear_focus(&mut self) {
        while let Some(node) = self.focus_stack.pop() {
            self.notify(&FocusEvent::FocusLost {
                node_id: node.id,
                capability: node.active_capability,
            });
        }
    }

    /// 恢复上一个焦点
    pub fn restore_previous_focus(&mut self) -> bool {
        // 从历史中找最近的可聚焦节点
        for &node_id in self.focus_history.iter().rev() {
            if self.focusables.contains_key(&node_id) {
                return self.request_focus_simple(node_id);
            }
        }
        false
    }

    /// 检查节点是否聚焦
    pub fn is_focused(&self, node_id: u32) -> bool {
        self.focus_stack
            .last()
            .map_or(false, |n| n.id == node_id)
    }

    /// 获取当前聚焦的节点
    pub fn current_focus(&self) -> Option<&FocusNode> {
        self.focus_stack.last()
    }

    /// 获取当前聚焦的节点 ID
    pub fn focused_id(&self) -> u32 {
        self.focus_stack.last().map_or(0, |n| n.id)
    }

    /// 检查节点是否有特定能力
    pub fn has_capability(&self, node_id: u32, cap: FocusCapability) -> bool {
        self.focusables
            .get(&node_id)
            .map_or(false, |n| n.capabilities.contains(&cap))
    }

    /// 添加焦点监听器
    pub fn add_listener(&mut self, listener: FocusListener) {
        self.listeners.push(listener);
    }

    /// 移除焦点监听器（简单实现：清空所有）
    pub fn clear_listeners(&mut self) {
        self.listeners.clear();
    }

    /// 通知监听器
    fn notify(&self, event: &FocusEvent) {
        for listener in &self.listeners {
            listener(event);
        }
    }

    /// 添加到历史
    fn add_to_history(&mut self, node_id: u32) {
        // 移除重复项
        self.focus_history.retain(|&id| id != node_id);
        // 添加到末尾（最新的）
        self.focus_history.push(node_id);
        // 限制长度
        if self.focus_history.len() > self.max_history {
            self.focus_history.remove(0);
        }
    }
}

impl Default for FocusManager {
    fn default() -> Self {
        Self::new()
    }
}

// 全局焦点管理器实例（WASM 单线程）
thread_local! {
    static FOCUS_MANAGER: RefCell<FocusManager> = RefCell::new(FocusManager::new());
}

/// 获取焦点管理器的便利函数
pub fn with_focus_manager<F, R>(f: F) -> R
where
    F: FnOnce(&mut FocusManager) -> R,
{
    FOCUS_MANAGER.with(|fm| f(&mut fm.borrow_mut()))
}

/// 便利函数：请求焦点
pub fn request_focus(node_id: u32, capability: FocusCapability) -> bool {
    with_focus_manager(|fm| fm.request_focus(node_id, capability))
}

/// 便利函数：简化请求焦点
pub fn request_focus_simple(node_id: u32) -> bool {
    with_focus_manager(|fm| fm.request_focus_simple(node_id))
}

/// 便利函数：清除焦点
pub fn clear_focus() {
    with_focus_manager(|fm| fm.clear_focus())
}

/// 便利函数：获取当前焦点 ID
pub fn focused_id() -> u32 {
    with_focus_manager(|fm| fm.focused_id())
}

/// 便利函数：检查是否聚焦
pub fn is_focused(node_id: u32) -> bool {
    with_focus_manager(|fm| fm.is_focused(node_id))
}

/// 便利函数：注册可聚焦节点
pub fn register_focusable(node_id: u32, capabilities: Vec<FocusCapability>) {
    with_focus_manager(|fm| fm.register(node_id, capabilities))
}

/// 便利函数：注销节点
pub fn unregister_focusable(node_id: u32) {
    with_focus_manager(|fm| fm.unregister(node_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_focus() {
        let mut fm = FocusManager::new();
        fm.register(1, vec![FocusCapability::Keyboard]);
        fm.register(2, vec![FocusCapability::Activatable]);

        assert!(fm.request_focus_simple(1));
        assert_eq!(fm.focused_id(), 1);

        // 重复请求不应触发变化
        assert!(!fm.request_focus_simple(1));

        // 切换到另一个
        assert!(fm.request_focus_simple(2));
        assert_eq!(fm.focused_id(), 2);
    }

    #[test]
    fn test_clear_focus() {
        let mut fm = FocusManager::new();
        fm.register(1, vec![FocusCapability::Keyboard]);
        fm.request_focus_simple(1);

        assert_eq!(fm.focused_id(), 1);
        fm.clear_focus();
        assert_eq!(fm.focused_id(), 0);
    }

    #[test]
    fn test_capability_check() {
        let mut fm = FocusManager::new();
        fm.register(1, vec![FocusCapability::Keyboard, FocusCapability::Selectable]);

        assert!(fm.has_capability(1, FocusCapability::Keyboard));
        assert!(fm.has_capability(1, FocusCapability::Selectable));
        assert!(!fm.has_capability(1, FocusCapability::Activatable));
    }
}
