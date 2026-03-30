// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Input Proxy 共享输入协议
//!
//! 定义标准化的输入事件格式和环形缓冲区，用于宿主环境与 WASM 之间的高频输入交互。

/// 输入事件类型
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEventType {
    /// 指针按下
    PointerDown = 0,
    /// 指针移动
    PointerMove = 1,
    /// 指针抬起
    PointerUp = 2,
    /// 指针取消（系统中断，如来电）
    PointerCancel = 3,
    /// 鼠标滚轮
    MouseWheel = 4,
    /// 按键按下
    KeyDown = 5,
    /// 按键抬起
    KeyUp = 6,
}

impl InputEventType {
    /// 从 u8 转换（用于从缓冲区读取）
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::PointerDown),
            1 => Some(Self::PointerMove),
            2 => Some(Self::PointerUp),
            3 => Some(Self::PointerCancel),
            4 => Some(Self::MouseWheel),
            5 => Some(Self::KeyDown),
            6 => Some(Self::KeyUp),
            _ => None,
        }
    }
}

/// 原始输入事件
/// 
/// 固定 32 字节大小，确保可预测内存布局和高效传输
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RawInputEvent {
    /// 微秒级时间戳（从系统启动开始）
    pub timestamp: u64,
    /// 事件类型
    pub event_type: InputEventType,
    /// 多点触控 ID（单指为 0）
    pub pointer_id: u32,
    /// 世界坐标 X（已考虑 DPI 缩放）
    pub x: f32,
    /// 世界坐标 Y（已考虑 DPI 缩放）
    pub y: f32,
    /// 按压力度（0.0 ~ 1.0）
    pub pressure: f32,
    /// X 方向增量（用于滚动或 Pan）
    pub delta_x: f32,
    /// Y 方向增量（用于滚动或 Pan）
    pub delta_y: f32,
    /// 宿主侧预计算的命中节点 ID（0 表示未命中）
    pub target_node_id: u32,
    /// 扩展标志位（用于未来扩展，如：是否来自手写笔）
    pub flags: u32,
}

impl Default for RawInputEvent {
    fn default() -> Self {
        Self {
            timestamp: 0,
            event_type: InputEventType::PointerDown,
            pointer_id: 0,
            x: 0.0,
            y: 0.0,
            pressure: 1.0,
            delta_x: 0.0,
            delta_y: 0.0,
            target_node_id: 0,
            flags: 0,
        }
    }
}

/// 缓冲区容量：100 个事件（约 3.2KB）
/// 
/// 在 120Hz 采样率下可缓存约 830ms 的事件，足够应对帧率波动
pub const INPUT_BUFFER_CAPACITY: usize = 100;

/// 输入事件环形缓冲区
/// 
/// 使用单生产者单消费者模型：
/// - 生产者：宿主侧输入线程
/// - 消费者：WASM 逻辑线程
/// 
/// 使用 wrapping_add 实现无锁环形缓冲区
#[repr(C)]
pub struct InputBuffer {
    /// 写入位置（宿主侧单调递增）
    pub write_idx: u32,
    /// 读取位置（WASM 侧单调递增）
    pub read_idx: u32,
    /// 溢出计数（调试使用）
    pub overflow_count: u32,
    /// 保留字段（用于未来扩展，如：最后溢出时间）
    _reserved: u32,
    /// 事件存储数组
    pub events: [RawInputEvent; INPUT_BUFFER_CAPACITY],
}

impl InputBuffer {
    /// 创建空的输入缓冲区
    pub const fn new() -> Self {
        Self {
            write_idx: 0,
            read_idx: 0,
            overflow_count: 0,
            _reserved: 0,
            events: [RawInputEvent {
                timestamp: 0,
                event_type: InputEventType::PointerDown,
                pointer_id: 0,
                x: 0.0,
                y: 0.0,
                pressure: 1.0,
                delta_x: 0.0,
                delta_y: 0.0,
                target_node_id: 0,
                flags: 0,
            }; INPUT_BUFFER_CAPACITY],
        }
    }

    /// 检查缓冲区是否为空
    pub fn is_empty(&self) -> bool {
        self.read_idx == self.write_idx
    }

    /// 检查缓冲区是否已满
    pub fn is_full(&self) -> bool {
        self.write_idx - self.read_idx >= INPUT_BUFFER_CAPACITY as u32
    }

    /// 当前事件数量
    pub fn len(&self) -> usize {
        (self.write_idx - self.read_idx) as usize
    }

    /// 压入事件（宿主侧调用）
    /// 
    /// 返回 true 表示成功，false 表示缓冲区已满（事件被丢弃）
    pub fn push(&mut self, event: RawInputEvent) -> bool {
        if self.is_full() {
            self.overflow_count += 1;
            return false;
        }
        let idx = (self.write_idx % INPUT_BUFFER_CAPACITY as u32) as usize;
        self.events[idx] = event;
        self.write_idx += 1;
        true
    }

    /// 弹出事件（WASM 侧调用）
    /// 
    /// 返回 None 表示缓冲区为空
    pub fn pop(&mut self) -> Option<RawInputEvent> {
        if self.is_empty() {
            return None;
        }
        let idx = (self.read_idx % INPUT_BUFFER_CAPACITY as u32) as usize;
        let event = self.events[idx];
        self.read_idx += 1;
        Some(event)
    }

    /// 批量读取所有可用事件
    /// 
    /// 用于帧开始时一次性处理所有累积事件
    pub fn drain(&mut self) -> InputBufferDrainIterator<'_> {
        InputBufferDrainIterator { buffer: self }
    }

    /// 查看下一个事件（不弹出）
    pub fn peek(&self) -> Option<&RawInputEvent> {
        if self.is_empty() {
            return None;
        }
        let idx = (self.read_idx % INPUT_BUFFER_CAPACITY as u32) as usize;
        Some(&self.events[idx])
    }

    /// 清空缓冲区
    pub fn clear(&mut self) {
        self.read_idx = self.write_idx;
    }

    /// 获取溢出次数并重置计数器
    pub fn take_overflow_count(&mut self) -> u32 {
        let count = self.overflow_count;
        self.overflow_count = 0;
        count
    }
}

/// 输入缓冲区批量读取迭代器
pub struct InputBufferDrainIterator<'a> {
    buffer: &'a mut InputBuffer,
}

impl<'a> Iterator for InputBufferDrainIterator<'a> {
    type Item = RawInputEvent;

    fn next(&mut self) -> Option<Self::Item> {
        self.buffer.pop()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.buffer.len();
        (len, Some(len))
    }
}

impl<'a> ExactSizeIterator for InputBufferDrainIterator<'a> {}

/// 输入事件标志位常量
pub mod input_flags {
    /// 是否来自手写笔
    pub const STYLUS: u32 = 1 << 0;
    /// 是否来自橡皮擦（手写笔翻转）
    pub const ERASER: u32 = 1 << 1;
    /// 是否为主按钮（鼠标左键/主手指）
    pub const PRIMARY: u32 = 1 << 2;
    /// 是否为次按钮（鼠标右键/次手指）
    pub const SECONDARY: u32 = 1 << 3;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_buffer_basic() {
        let mut buffer = InputBuffer::new();
        
        // 初始状态
        assert!(buffer.is_empty());
        assert!(!buffer.is_full());
        assert_eq!(buffer.len(), 0);
        
        // 压入事件
        let event = RawInputEvent::default();
        assert!(buffer.push(event));
        assert!(!buffer.is_empty());
        assert_eq!(buffer.len(), 1);
        
        // 弹出事件
        let popped = buffer.pop();
        assert!(popped.is_some());
        assert!(buffer.is_empty());
    }

    #[test]
    fn test_input_buffer_wrap_around() {
        let mut buffer = InputBuffer::new();
        
        // 写入 100 个事件
        for i in 0..INPUT_BUFFER_CAPACITY {
            let mut event = RawInputEvent::default();
            event.pointer_id = i as u32;
            assert!(buffer.push(event));
        }
        
        assert!(buffer.is_full());
        assert_eq!(buffer.len(), INPUT_BUFFER_CAPACITY);
        
        // 读取 50 个
        for _ in 0..50 {
            buffer.pop();
        }
        
        assert_eq!(buffer.len(), INPUT_BUFFER_CAPACITY - 50);
        
        // 再写入 50 个（测试环绕）
        for i in 0..50 {
            let mut event = RawInputEvent::default();
            event.pointer_id = (i + 100) as u32;
            assert!(buffer.push(event));
        }
        
        assert!(buffer.is_full());
    }

    #[test]
    fn test_input_buffer_overflow() {
        let mut buffer = InputBuffer::new();
        
        // 填满缓冲区
        for _ in 0..INPUT_BUFFER_CAPACITY {
            assert!(buffer.push(RawInputEvent::default()));
        }
        
        // 下一个应该失败
        assert!(!buffer.push(RawInputEvent::default()));
        assert_eq!(buffer.overflow_count, 1);
        
        // 再试一次
        assert!(!buffer.push(RawInputEvent::default()));
        assert_eq!(buffer.overflow_count, 2);
    }

    #[test]
    fn test_input_buffer_drain() {
        let mut buffer = InputBuffer::new();
        
        // 写入 10 个事件
        for i in 0..10 {
            let mut event = RawInputEvent::default();
            event.pointer_id = i as u32;
            buffer.push(event);
        }
        
        // 批量读取
        let events: Vec<_> = buffer.drain().collect();
        assert_eq!(events.len(), 10);
        assert!(buffer.is_empty());
        
        // 验证顺序
        for (i, event) in events.iter().enumerate() {
            assert_eq!(event.pointer_id, i as u32);
        }
    }

    #[test]
    fn test_event_type_from_u8() {
        assert_eq!(InputEventType::from_u8(0), Some(InputEventType::PointerDown));
        assert_eq!(InputEventType::from_u8(1), Some(InputEventType::PointerMove));
        assert_eq!(InputEventType::from_u8(2), Some(InputEventType::PointerUp));
        assert_eq!(InputEventType::from_u8(3), Some(InputEventType::PointerCancel));
        assert_eq!(InputEventType::from_u8(99), None);
    }
}
