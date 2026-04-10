// crates/dyxel-view/src/focus.rs
//! 焦点管理系统
//!
//! 提供统一的焦点管理能力，支持所有 View 组件。

pub use crate::focus_manager::{
    clear_focus, focused_id, is_focused, register_focusable, request_focus, request_focus_simple,
    unregister_focusable, with_focus_manager, FocusCapability, FocusEvent, FocusManager, FocusNode,
};

use std::cell::RefCell;
use std::rc::Rc;

/// 可聚焦视图 trait
///
/// 实现此 trait 的组件可以参与焦点管理系统。
/// 组件可以选择实现部分或全部焦点能力。
pub trait FocusableView: crate::BaseView {
    /// 获取该视图支持的焦点能力列表
    fn focus_capabilities(&self) -> Vec<FocusCapability>;

    /// 当视图获得焦点时调用
    fn on_focus(&self, capability: FocusCapability) {
        let _ = capability;
    }

    /// 当视图失去焦点时调用
    fn on_blur(&self, capability: FocusCapability) {
        let _ = capability;
    }

    /// 注册到焦点管理器
    fn register_focus(&self) {
        let caps = self.focus_capabilities();
        if !caps.is_empty() {
            register_focusable(self.node_id(), caps);
        }
    }

    /// 从焦点管理器注销
    fn unregister_focus(&self) {
        unregister_focusable(self.node_id());
    }

    /// 请求焦点（使用默认能力）
    fn request_focus(&self) -> bool {
        request_focus_simple(self.node_id())
    }

    /// 请求特定焦点能力
    fn request_focus_with_capability(&self, capability: FocusCapability) -> bool {
        request_focus(self.node_id(), capability)
    }

    /// 检查当前是否聚焦
    fn is_focused(&self) -> bool {
        is_focused(self.node_id())
    }

    /// 处理点击事件（自动请求焦点）
    fn handle_tap_for_focus(&self) {
        let caps = self.focus_capabilities();
        for cap in &caps {
            if cap.auto_focus_on_tap() {
                self.request_focus_with_capability(*cap);
                break;
            }
        }
    }
}

/// 扩展 BaseView 以支持焦点操作
pub trait FocusExt: crate::BaseView {
    /// 为该视图启用焦点能力
    fn enable_focus(self, capabilities: Vec<FocusCapability>) -> Self
    where
        Self: Sized,
    {
        register_focusable(self.node_id(), capabilities);
        self
    }

    /// 设置获得焦点时的回调
    fn on_focus<F>(self, handler: F) -> Self
    where
        Self: Sized + 'static,
        F: FnMut(FocusCapability) + 'static,
    {
        let id = self.node_id();
        let handler = Rc::new(RefCell::new(handler));
        with_focus_manager(|fm| {
            fm.add_listener(Box::new(move |event| {
                if let FocusEvent::FocusGained { node_id, capability } = event {
                    if *node_id == id {
                        handler.borrow_mut()(*capability);
                    }
                }
            }));
        });
        self
    }

    /// 设置失去焦点时的回调
    fn on_blur<F>(self, handler: F) -> Self
    where
        Self: Sized + 'static,
        F: FnMut(FocusCapability) + 'static,
    {
        let id = self.node_id();
        let handler = Rc::new(RefCell::new(handler));
        with_focus_manager(|fm| {
            fm.add_listener(Box::new(move |event| {
                if let FocusEvent::FocusLost { node_id, capability } = event {
                    if *node_id == id {
                        handler.borrow_mut()(*capability);
                    }
                }
            }));
        });
        self
    }
}

impl<T: crate::BaseView> FocusExt for T {}
