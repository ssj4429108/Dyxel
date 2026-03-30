// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Gesture synthesizer - Convert raw input events to high-level gestures
//!
//! Implement state machine pattern to recognize Tap, LongPress, Pan, etc. gestures.

use dyxel_shared::{InputEventType, RawInputEvent};

/// Gesture types
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gesture {
    /// Tap
    Tap {
        node_id: u32,
        x: f32,
        y: f32,
    },
    /// Long press
    LongPress {
        node_id: u32,
        x: f32,
        y: f32,
    },
    /// Pan start
    PanStart {
        node_id: u32,
        x: f32,
        y: f32,
    },
    /// Pan update
    PanUpdate {
        node_id: u32,
        x: f32,
        y: f32,
        delta_x: f32,
        delta_y: f32,
    },
    /// Pan end
    PanEnd {
        node_id: u32,
        x: f32,
        y: f32,
    },
}

impl Gesture {
    /// Gesture getters目标节点 ID
    pub fn node_id(&self) -> u32 {
        match self {
            Gesture::Tap { node_id, .. } => *node_id,
            Gesture::LongPress { node_id, .. } => *node_id,
            Gesture::PanStart { node_id, .. } => *node_id,
            Gesture::PanUpdate { node_id, .. } => *node_id,
            Gesture::PanEnd { node_id, .. } => *node_id,
        }
    }

    /// Gesture gettersPosition
    pub fn position(&self) -> (f32, f32) {
        match self {
            Gesture::Tap { x, y, .. } => (*x, *y),
            Gesture::LongPress { x, y, .. } => (*x, *y),
            Gesture::PanStart { x, y, .. } => (*x, *y),
            Gesture::PanUpdate { x, y, .. } => (*x, *y),
            Gesture::PanEnd { x, y, .. } => (*x, *y),
        }
    }
}

/// Gesture recognition config
#[derive(Debug, Clone, Copy)]
pub struct GestureConfig {
    /// Tap 超时时间（微秒）
    pub tap_timeout_us: u64,
    /// 触摸偏差阈值（像素）
    pub touch_slop: f32,
    /// Long press超时时间（微秒）
    pub long_press_timeout_us: u64,
}

impl Default for GestureConfig {
    fn default() -> Self {
        Self {
            tap_timeout_us: 300_000,      // 300ms
            touch_slop: 10.0,              // 10px
            long_press_timeout_us: 500_000, // 500ms
        }
    }
}

/// Single pointer state
#[derive(Debug, Clone, Copy)]
enum PointerState {
    /// Idle state
    Idle,
    /// Down state
    Down {
        node_id: u32,
        start_x: f32,
        start_y: f32,
        down_time: u64,
    },
    /// Panning state
    Panning {
        node_id: u32,
        last_x: f32,
        last_y: f32,
    },
}

/// Gesture recognizer
///
/// Use state machine pattern to process input events and recognize gestures.
pub struct GestureRecognizer {
    config: GestureConfig,
    state: PointerState,
}

impl GestureRecognizer {
    /// 创建新的Gesture recognizer
    pub fn new() -> Self {
        Self {
            config: GestureConfig::default(),
            state: PointerState::Idle,
        }
    }

    /// 使用自定义配置创建
    pub fn with_config(config: GestureConfig) -> Self {
        Self {
            config,
            state: PointerState::Idle,
        }
    }

    /// Process raw input events, return recognized gestures
    ///
    /// # 状态转换
    /// - Idle + PointerDown → Down
    /// - Down + PointerMove (超阈值) → PanStart
    /// - Down + PointerUp (时间内) → Tap
    /// - Down + PointerUp (超时) → None
    /// - Panning + PointerMove → PanUpdate
    /// - Panning + PointerUp → PanEnd
    pub fn process_event(&mut self, event: &RawInputEvent) -> Option<Gesture> {
        // 只处理第一指针（pointer_id == 0）
        if event.pointer_id != 0 {
            return None;
        }

        // 使用临时变量避免借用冲突
        let current_state = std::mem::replace(&mut self.state, PointerState::Idle);
        
        let (new_state, gesture) = match (current_state, event.event_type) {
            // Idle → Down
            (PointerState::Idle, InputEventType::PointerDown) => {
                let new_state = PointerState::Down {
                    node_id: event.target_node_id,
                    start_x: event.x,
                    start_y: event.y,
                    down_time: event.timestamp,
                };
                (new_state, None)
            }

            // Down → Panning (超出 touch slop)
            (
                PointerState::Down {
                    node_id,
                    start_x,
                    start_y,
                    down_time,
                },
                InputEventType::PointerMove,
            ) => {
                let dx = event.x - start_x;
                let dy = event.y - start_y;

                if dx.abs() > self.config.touch_slop || dy.abs() > self.config.touch_slop {
                    let new_state = PointerState::Panning {
                        node_id,
                        last_x: event.x,
                        last_y: event.y,
                    };
                    let gesture = Gesture::PanStart {
                        node_id,
                        x: event.x,
                        y: event.y,
                    };
                    (new_state, Some(gesture))
                } else {
                    // 保持在 Down 状态
                    let new_state = PointerState::Down {
                        node_id,
                        start_x,
                        start_y,
                        down_time,
                    };
                    (new_state, None)
                }
            }

            // Down → Tap (在时间阈值内抬起)
            (
                PointerState::Down {
                    node_id,
                    start_x: _,
                    start_y: _,
                    down_time,
                },
                InputEventType::PointerUp,
            ) => {
                let elapsed = event.timestamp - down_time;
                let gesture = if elapsed < self.config.tap_timeout_us {
                    Some(Gesture::Tap {
                        node_id,
                        x: event.x,
                        y: event.y,
                    })
                } else {
                    None
                };
                (PointerState::Idle, gesture)
            }

            // Panning → PanUpdate
            (
                PointerState::Panning {
                    node_id,
                    last_x,
                    last_y,
                },
                InputEventType::PointerMove,
            ) => {
                let delta_x = event.x - last_x;
                let delta_y = event.y - last_y;

                let new_state = PointerState::Panning {
                    node_id,
                    last_x: event.x,
                    last_y: event.y,
                };
                let gesture = Gesture::PanUpdate {
                    node_id,
                    x: event.x,
                    y: event.y,
                    delta_x,
                    delta_y,
                };
                (new_state, Some(gesture))
            }

            // Panning → PanEnd
            (
                PointerState::Panning { node_id, .. },
                InputEventType::PointerUp,
            ) => {
                let gesture = Gesture::PanEnd {
                    node_id,
                    x: event.x,
                    y: event.y,
                };
                (PointerState::Idle, Some(gesture))
            }

            // Cancel → 重置状态
            (_, InputEventType::PointerCancel) => {
                (PointerState::Idle, None)
            }

            // 其他组合：保持原状态
            (state, _) => (state, None),
        };
        
        self.state = new_state;
        gesture
    }

    /// 检查是否处于Down state
    pub fn is_down(&self) -> bool {
        matches!(self.state, PointerState::Down { .. })
    }

    /// 检查是否正在平移
    pub fn is_panning(&self) -> bool {
        matches!(self.state, PointerState::Panning { .. })
    }

    /// 重置状态机
    pub fn reset(&mut self) {
        self.state = PointerState::Idle;
    }

    /// 获取当前状态（用于调试）
    #[cfg(debug_assertions)]
    pub fn state_name(&self) -> &'static str {
        match self.state {
            PointerState::Idle => "Idle",
            PointerState::Down { .. } => "Down",
            PointerState::Panning { .. } => "Panning",
        }
    }
}

impl Default for GestureRecognizer {
    fn default() -> Self {
        Self::new()
    }
}

/// Frame-level event coalescing
///
/// 合并高频 PointerMove 事件，减少处理器负担
pub fn coalesce_events(events: Vec<RawInputEvent>) -> Vec<RawInputEvent> {
    if events.len() < 2 {
        return events;
    }

    let mut result = Vec::with_capacity(events.len());
    let mut i = 0;

    while i < events.len() {
        let current = &events[i];

        // 检查是否可以与后续事件合并
        if current.event_type == InputEventType::PointerMove {
            let mut last_idx = i;
            let mut accumulated_dx = current.delta_x;
            let mut accumulated_dy = current.delta_y;

            for j in (i + 1)..events.len() {
                let next = &events[j];
                if next.event_type == InputEventType::PointerMove
                    && next.pointer_id == current.pointer_id
                    && next.target_node_id == current.target_node_id
                {
                    last_idx = j;
                    accumulated_dx += next.delta_x;
                    accumulated_dy += next.delta_y;
                } else {
                    break;
                }
            }

            if last_idx > i {
                // 创建合并后的事件
                let mut merged = *current;
                merged.delta_x = accumulated_dx;
                merged.delta_y = accumulated_dy;
                // 使用最后一个事件的Position
                merged.x = events[last_idx].x;
                merged.y = events[last_idx].y;
                result.push(merged);
                i = last_idx + 1;
                continue;
            }
        }

        result.push(*current);
        i += 1;
    }

    result
}

/// Batch process input events
///
/// Merge first then recognize gestures
pub fn process_event_batch(
    events: Vec<RawInputEvent>,
    recognizer: &mut GestureRecognizer,
) -> Vec<Gesture> {
    let merged = coalesce_events(events);
    let mut gestures = Vec::new();

    for event in merged {
        if let Some(gesture) = recognizer.process_event(&event) {
            gestures.push(gesture);
        }
    }

    gestures
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_event(event_type: InputEventType, x: f32, y: f32) -> RawInputEvent {
        RawInputEvent {
            timestamp: 0,
            event_type,
            pointer_id: 0,
            x,
            y,
            pressure: 1.0,
            delta_x: 0.0,
            delta_y: 0.0,
            target_node_id: 1,
            flags: 0,
        }
    }

    #[test]
    fn test_tap_recognition() {
        let mut recognizer = GestureRecognizer::new();

        // PointerDown
        let down = create_event(InputEventType::PointerDown, 100.0, 100.0);
        assert!(recognizer.process_event(&down).is_none());
        assert!(recognizer.is_down());

        // PointerUp (快速)
        let up = RawInputEvent {
            timestamp: 100_000, // 100ms < 300ms
            event_type: InputEventType::PointerUp,
            ..down
        };
        let gesture = recognizer.process_event(&up);
        assert!(gesture.is_some());
        assert!(matches!(gesture.unwrap(), Gesture::Tap { .. }));
        assert!(!recognizer.is_down());
    }

    #[test]
    fn test_tap_timeout() {
        let mut recognizer = GestureRecognizer::new();

        // PointerDown
        let down = create_event(InputEventType::PointerDown, 100.0, 100.0);
        recognizer.process_event(&down);

        // PointerUp (超时)
        let up = RawInputEvent {
            timestamp: 400_000, // 400ms > 300ms
            event_type: InputEventType::PointerUp,
            ..down
        };
        let gesture = recognizer.process_event(&up);
        assert!(gesture.is_none()); // 超时，不是 Tap
    }

    #[test]
    fn test_pan_recognition() {
        let mut recognizer = GestureRecognizer::new();

        // PointerDown
        let down = create_event(InputEventType::PointerDown, 100.0, 100.0);
        recognizer.process_event(&down);

        // PointerMove (小幅度，在 slop 内)
        let move1 = RawInputEvent {
            event_type: InputEventType::PointerMove,
            x: 105.0,
            y: 105.0,
            ..down
        };
        let gesture = recognizer.process_event(&move1);
        assert!(gesture.is_none()); // 还在 slop 内

        // PointerMove (超出 slop)
        let move2 = RawInputEvent {
            event_type: InputEventType::PointerMove,
            x: 120.0, // 20px > 10px slop
            y: 100.0,
            ..down
        };
        let gesture = recognizer.process_event(&move2);
        assert!(matches!(gesture.unwrap(), Gesture::PanStart { .. }));
        assert!(recognizer.is_panning());

        // PointerMove (继续平移)
        let move3 = RawInputEvent {
            event_type: InputEventType::PointerMove,
            x: 125.0,
            y: 100.0,
            delta_x: 5.0,
            delta_y: 0.0,
            ..down
        };
        let gesture = recognizer.process_event(&move3);
        assert!(matches!(gesture.unwrap(), Gesture::PanUpdate { .. }));

        // PointerUp
        let up = RawInputEvent {
            event_type: InputEventType::PointerUp,
            x: 125.0,
            y: 100.0,
            ..down
        };
        let gesture = recognizer.process_event(&up);
        assert!(matches!(gesture.unwrap(), Gesture::PanEnd { .. }));
        assert!(!recognizer.is_panning());
    }

    #[test]
    fn test_coalesce_moves() {
        let events = vec![
            create_event(InputEventType::PointerDown, 0.0, 0.0),
            RawInputEvent {
                event_type: InputEventType::PointerMove,
                x: 5.0,
                y: 0.0,
                delta_x: 5.0,
                delta_y: 0.0,
                ..create_event(InputEventType::PointerMove, 5.0, 0.0)
            },
            RawInputEvent {
                event_type: InputEventType::PointerMove,
                x: 10.0,
                y: 0.0,
                delta_x: 5.0,
                delta_y: 0.0,
                ..create_event(InputEventType::PointerMove, 10.0, 0.0)
            },
            RawInputEvent {
                event_type: InputEventType::PointerMove,
                x: 15.0,
                y: 0.0,
                delta_x: 5.0,
                delta_y: 0.0,
                ..create_event(InputEventType::PointerMove, 15.0, 0.0)
            },
            create_event(InputEventType::PointerUp, 15.0, 0.0),
        ];

        let coalesced = coalesce_events(events);
        // Down + 合并后的 Move + Up = 3 个事件
        assert_eq!(coalesced.len(), 3);
        
        // 检查合并后的 Move 事件
        let merged_move = &coalesced[1];
        assert_eq!(merged_move.x, 15.0);
        assert_eq!(merged_move.delta_x, 15.0); // 5+5+5
    }
}
