// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Input Proxy - 宿主侧输入代理
//!
//! 负责将原生输入事件转换为标准格式，完成命中检测，并压入共享缓冲区。

use kurbo::{Affine, Point, Rect as KurboRect, Vec2};
use std::collections::HashMap;

use dyxel_shared::{
    InputBuffer, InputEventType, RawInputEvent, SharedBuffer,
};

use crate::state::{SharedState, ViewNode};

/// 原生输入事件类型（从平台接收）
#[derive(Debug, Clone, Copy)]
pub enum NativeInputType {
    TouchDown,
    TouchMove,
    TouchUp,
    TouchCancel,
    MouseWheel { delta_x: f32, delta_y: f32 },
}

/// 多点触控状态跟踪
#[derive(Debug, Clone)]
struct PointerState {
    start_x: f32,
    start_y: f32,
    start_time: u64,
    last_x: f32,
    last_y: f32,
    target_node_id: u32,
    is_panning: bool,
}

/// 输入代理配置
#[derive(Debug, Clone)]
pub struct InputProxyConfig {
    /// 热区扩展值（dp）
    pub hit_area_expansion: f32,
    /// 最小触摸目标大小（dp）
    pub min_touch_target: f32,
    /// 触摸偏差阈值（判定为 Pan 的像素距离）
    pub touch_slop: f32,
    /// DPI 缩放因子
    pub dpi_scale: f32,
}

impl Default for InputProxyConfig {
    fn default() -> Self {
        Self {
            hit_area_expansion: 8.0,
            min_touch_target: 44.0,
            touch_slop: 10.0,
            dpi_scale: 1.0,
        }
    }
}

/// 输入代理
///
/// 处理流程：
/// 1. 接收原生输入事件
/// 2. 坐标投影（屏幕 → 世界）
/// 3. 热区扩展命中检测
/// 4. 压入共享缓冲区
pub struct InputProxy {
    config: InputProxyConfig,
    /// 屏幕到世界坐标的变换矩阵
    screen_to_world: Affine,
    /// 多点触控状态（pointer_id → state）
    pointer_states: HashMap<u32, PointerState>,
    /// 当前时间戳（微秒）
    current_time: u64,
}

impl InputProxy {
    /// 创建新的输入代理
    pub fn new(config: InputProxyConfig) -> Self {
        Self {
            config,
            screen_to_world: Affine::IDENTITY,
            pointer_states: HashMap::new(),
            current_time: 0,
        }
    }

    /// 设置坐标变换矩阵
    ///
    /// 通常在渲染视图变化时调用（如缩放、平移）
    pub fn set_transform(&mut self, transform: Affine) {
        self.screen_to_world = transform;
    }

    /// 设置 DPI 缩放
    pub fn set_dpi_scale(&mut self, scale: f32) {
        self.config.dpi_scale = scale;
    }

    /// 处理原生输入事件
    ///
    /// 这是主要入口点，由平台层（Android/iOS/macOS）调用
    pub fn handle_native_event(
        &mut self,
        native_type: NativeInputType,
        pointer_id: u32,
        x: f32,
        y: f32,
        pressure: f32,
        shared_buffer: &mut SharedBuffer,
        state: &SharedState,
    ) {
        // 更新时间戳
        self.current_time = Self::current_time_micros();

        // 坐标投影：屏幕 → 世界
        let world_pos = self.project_to_world(x, y);

        // 根据事件类型处理
        match native_type {
            NativeInputType::TouchDown => {
                self.handle_pointer_down(
                    pointer_id,
                    world_pos,
                    pressure,
                    shared_buffer,
                    state,
                );
            }
            NativeInputType::TouchMove => {
                self.handle_pointer_move(
                    pointer_id,
                    world_pos,
                    pressure,
                    shared_buffer,
                );
            }
            NativeInputType::TouchUp => {
                self.handle_pointer_up(
                    pointer_id,
                    world_pos,
                    shared_buffer,
                );
            }
            NativeInputType::TouchCancel => {
                self.handle_pointer_cancel(pointer_id, shared_buffer);
            }
            NativeInputType::MouseWheel { delta_x, delta_y } => {
                self.handle_mouse_wheel(
                    pointer_id,
                    world_pos,
                    delta_x,
                    delta_y,
                    shared_buffer,
                    state,
                );
            }
        }
    }

    /// 处理指针按下
    fn handle_pointer_down(
        &mut self,
        pointer_id: u32,
        world_pos: Point,
        pressure: f32,
        shared_buffer: &mut SharedBuffer,
        state: &SharedState,
    ) {
        // 命中检测（带热区扩展）
        let target_id = self
            .hit_test_with_expansion(world_pos, state)
            .unwrap_or(0);

        // 记录指针状态
        let pointer_state = PointerState {
            start_x: world_pos.x as f32,
            start_y: world_pos.y as f32,
            start_time: self.current_time,
            last_x: world_pos.x as f32,
            last_y: world_pos.y as f32,
            target_node_id: target_id,
            is_panning: false,
        };
        self.pointer_states.insert(pointer_id, pointer_state);

        // 创建事件并压入缓冲区
        let event = RawInputEvent {
            timestamp: self.current_time,
            event_type: InputEventType::PointerDown,
            pointer_id,
            x: world_pos.x as f32,
            y: world_pos.y as f32,
            pressure,
            delta_x: 0.0,
            delta_y: 0.0,
            target_node_id: target_id,
            flags: 0,
        };

        self.push_event(shared_buffer, event);
    }

    /// 处理指针移动
    fn handle_pointer_move(
        &mut self,
        pointer_id: u32,
        world_pos: Point,
        _pressure: f32,
        shared_buffer: &mut SharedBuffer,
    ) {
        let Some(state) = self.pointer_states.get_mut(&pointer_id) else {
            // 没有对应的按下事件，忽略
            return;
        };

        // 计算增量
        let delta_x = world_pos.x as f32 - state.last_x;
        let delta_y = world_pos.y as f32 - state.last_y;

        // 更新最后位置
        state.last_x = world_pos.x as f32;
        state.last_y = world_pos.y as f32;

        // 检查是否超过 Pan 阈值
        if !state.is_panning {
            let dx_from_start = world_pos.x as f32 - state.start_x;
            let dy_from_start = world_pos.y as f32 - state.start_y;
            let slop = self.config.touch_slop * self.config.dpi_scale;

            if dx_from_start.abs() > slop || dy_from_start.abs() > slop {
                state.is_panning = true;
            }
        }

        // 创建事件
        let event = RawInputEvent {
            timestamp: self.current_time,
            event_type: InputEventType::PointerMove,
            pointer_id,
            x: world_pos.x as f32,
            y: world_pos.y as f32,
            pressure: 1.0,
            delta_x,
            delta_y,
            target_node_id: state.target_node_id,
            flags: if state.is_panning { 1 } else { 0 },
        };

        self.push_event(shared_buffer, event);
    }

    /// 处理指针抬起
    fn handle_pointer_up(
        &mut self,
        pointer_id: u32,
        world_pos: Point,
        shared_buffer: &mut SharedBuffer,
    ) {
        let Some(state) = self.pointer_states.remove(&pointer_id) else {
            return;
        };

        let delta_x = world_pos.x as f32 - state.last_x;
        let delta_y = world_pos.y as f32 - state.last_y;

        let event = RawInputEvent {
            timestamp: self.current_time,
            event_type: InputEventType::PointerUp,
            pointer_id,
            x: world_pos.x as f32,
            y: world_pos.y as f32,
            pressure: 0.0,
            delta_x,
            delta_y,
            target_node_id: state.target_node_id,
            flags: if state.is_panning { 1 } else { 0 },
        };

        self.push_event(shared_buffer, event);
    }

    /// 处理指针取消
    fn handle_pointer_cancel(
        &mut self,
        pointer_id: u32,
        shared_buffer: &mut SharedBuffer,
    ) {
        let Some(state) = self.pointer_states.remove(&pointer_id) else {
            return;
        };

        let event = RawInputEvent {
            timestamp: self.current_time,
            event_type: InputEventType::PointerCancel,
            pointer_id,
            x: state.last_x,
            y: state.last_y,
            pressure: 0.0,
            delta_x: 0.0,
            delta_y: 0.0,
            target_node_id: state.target_node_id,
            flags: 0,
        };

        self.push_event(shared_buffer, event);
    }

    /// 处理鼠标滚轮
    fn handle_mouse_wheel(
        &mut self,
        pointer_id: u32,
        world_pos: Point,
        delta_x: f32,
        delta_y: f32,
        shared_buffer: &mut SharedBuffer,
        state: &SharedState,
    ) {
        let target_id = self
            .hit_test_with_expansion(world_pos, state)
            .unwrap_or(0);

        let event = RawInputEvent {
            timestamp: self.current_time,
            event_type: InputEventType::MouseWheel,
            pointer_id,
            x: world_pos.x as f32,
            y: world_pos.y as f32,
            pressure: 0.0,
            delta_x,
            delta_y,
            target_node_id: target_id,
            flags: 0,
        };

        self.push_event(shared_buffer, event);
    }

    /// 坐标投影：屏幕 → 世界
    fn project_to_world(&self, screen_x: f32, screen_y: f32) -> Point {
        let point = Point::new(screen_x as f64, screen_y as f64);
        self.screen_to_world * point
    }

    /// 热区扩展命中检测
    ///
    /// 对小尺寸节点自动扩展热区，提高移动端点击准确率
    fn hit_test_with_expansion(
        &self,
        point: Point,
        state: &SharedState,
    ) -> Option<u32> {
        let root_id = state.root_id?;
        self.hit_test_recursive(
            root_id,
            point,
            state,
            Vec2::ZERO,
        )
    }

    /// 递归命中检测（带热区扩展）
    fn hit_test_recursive(
        &self,
        id: u32,
        point: Point,
        state: &SharedState,
        parent_pos: Vec2,
    ) -> Option<u32> {
        let node = state.nodes.get(&id)?;
        let layout = state.taffy.layout(node.taffy_node).ok()?;

        let global_pos = parent_pos
            + Vec2::new(layout.location.x as f64, layout.location.y as f64);

        // 计算带热区扩展的命中矩形
        let expansion = (self.config.hit_area_expansion * self.config.dpi_scale) as f64;
        let min_target = (self.config.min_touch_target * self.config.dpi_scale) as f64;

        let width = (layout.size.width as f64).max(min_target) + expansion * 2.0;
        let height = (layout.size.height as f64).max(min_target) + expansion * 2.0;

        let hit_rect = KurboRect::from_origin_size(
            (global_pos.x - expansion, global_pos.y - expansion),
            (width, height),
        );

        // 优先检查子节点（从后往前，顶层优先）
        for &child_id in node.children.iter().rev() {
            if let Some(hit) =
                self.hit_test_recursive(child_id, point, state, global_pos)
            {
                return Some(hit);
            }
        }

        // 检查当前节点
        if hit_rect.contains(point) {
            // 检查是否有事件监听器
            if state.click_listeners.contains(&id) || has_other_handlers(id, state) {
                return Some(id);
            }
        }

        None
    }

    /// 压入事件到缓冲区
    fn push_event(&self, shared_buffer: &mut SharedBuffer, event: RawInputEvent) {
        if !shared_buffer.input_buffer.push(event) {
            log::warn!(
                "Input buffer overflow! Event dropped: {:?}",
                event.event_type
            );
        }
    }

    /// 获取当前时间戳（微秒）
    fn current_time_micros() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64
    }

    /// 获取并清除溢出计数
    pub fn take_overflow_count(&self, shared_buffer: &mut SharedBuffer) -> u32 {
        shared_buffer.input_buffer.take_overflow_count()
    }

    /// 检查是否有活跃指针
    pub fn has_active_pointers(&self) -> bool {
        !self.pointer_states.is_empty()
    }

    /// 获取活跃指针数量
    pub fn active_pointer_count(&self) -> usize {
        self.pointer_states.len()
    }
}

/// 检查节点是否有其他类型的事件处理器
///
/// 注：目前简化实现，未来可扩展为检查手势处理器等
fn has_other_handlers(id: u32, state: &SharedState) -> bool {
    // 目前只检查点击监听器
    // 未来可以检查：
    // - 手势处理器
    // - 滚动监听器
    // - 拖拽源/目标
    state.click_listeners.contains(&id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_proxy_creation() {
        let proxy = InputProxy::new(InputProxyConfig::default());
        assert!(!proxy.has_active_pointers());
        assert_eq!(proxy.active_pointer_count(), 0);
    }

    #[test]
    fn test_project_to_world_identity() {
        let proxy = InputProxy::new(InputProxyConfig::default());
        let world = proxy.project_to_world(100.0, 200.0);
        assert_eq!(world.x, 100.0);
        assert_eq!(world.y, 200.0);
    }

    #[test]
    fn test_hit_area_expansion_config() {
        let config = InputProxyConfig {
            hit_area_expansion: 16.0,
            min_touch_target: 48.0,
            ..Default::default()
        };
        assert_eq!(config.hit_area_expansion, 16.0);
        assert_eq!(config.min_touch_target, 48.0);
    }
}
