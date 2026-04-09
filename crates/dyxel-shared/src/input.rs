// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Input Proxy - Shared Input Protocol
//!
//! Defines standardized input event format and ring buffer for high-frequency
//! input interaction between host environment and WASM.

/// Input event types
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEventType {
    /// Pointer pressed
    PointerDown = 0,
    /// Pointer moved
    PointerMove = 1,
    /// Pointer released
    PointerUp = 2,
    /// Pointer cancelled (system interrupt like phone call)
    PointerCancel = 3,
    /// Mouse wheel scrolled
    MouseWheel = 4,
    /// Key pressed
    KeyDown = 5,
    /// Key released
    KeyUp = 6,
    /// Text input (processed character)
    TextInput = 7,
    /// IME composition in progress
    ImeComposition = 8,
    /// IME commit final text
    ImeCommit = 9,
}

impl InputEventType {
    /// Convert from u8 (used when reading from buffer)
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::PointerDown),
            1 => Some(Self::PointerMove),
            2 => Some(Self::PointerUp),
            3 => Some(Self::PointerCancel),
            4 => Some(Self::MouseWheel),
            5 => Some(Self::KeyDown),
            6 => Some(Self::KeyUp),
            7 => Some(Self::TextInput),
            8 => Some(Self::ImeComposition),
            9 => Some(Self::ImeCommit),
            _ => None,
        }
    }
}

/// Keyboard event data
/// Stored in RawInputEvent payload for KeyDown/KeyUp events
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KeyEventData {
    /// Key code (platform-specific virtual key code)
    pub key_code: u32,
    /// Character UTF-32 code point (if applicable)
    pub char_code: u32,
    /// Modifier keys state: bit0=shift, bit1=ctrl, bit2=alt, bit3=meta/cmd
    pub modifiers: u8,
    /// Repeat count for held keys
    pub repeat_count: u8,
    /// Reserved padding
    pub _padding: [u8; 2],
}

/// Text input event data
/// Stored in RawInputEvent payload for TextInput/ImeComposition/ImeCommit events
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct TextInputData {
    /// Text length in bytes (max 16 for inline storage)
    pub len: u8,
    /// UTF-8 text content (inline storage)
    pub text: [u8; 16],
    /// Cursor position after insertion
    pub cursor_pos: u8,
    /// Reserved padding
    pub _padding: [u8; 6],
}

/// Raw input event
///
/// Fixed 64-byte size for predictable memory layout and efficient transfer.
/// Fields are ordered to minimize padding while maintaining alignment.
///
/// NOTE: event_type is stored as u8 for guaranteed cross-platform compatibility
/// between Host (native) and Guest (WASM). Use InputEventType::from_u8() to convert.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RawInputEvent {
    /// Timestamp in microseconds (from system boot)
    pub timestamp: u64,
    /// Multi-touch ID (0 for single pointer, reused as key_id for keyboard)
    pub pointer_id: u32,
    /// Event type as u8 (use InputEventType::from_u8() to convert)
    pub event_type: u8,
    /// Padding to maintain alignment - reserved for future use
    pub _padding: [u8; 3],
    /// World coordinate X (DPI scaled) / or key_code for keyboard events
    pub x: f32,
    /// World coordinate Y (DPI scaled) / or char_code for keyboard events
    pub y: f32,
    /// Pressure value (0.0 ~ 1.0) / or modifiers for keyboard events
    pub pressure: f32,
    /// Delta X (for scroll or pan)
    pub delta_x: f32,
    /// Delta Y (for scroll or pan)
    pub delta_y: f32,
    /// Pre-calculated hit target node ID from host (0 = no hit)
    pub target_node_id: u32,
    /// Extension flags (for future use like stylus detection)
    /// bit0=stylus, bit1=eraser, bit2=primary, bit3=secondary
    /// For keyboard: bit0=shift, bit1=ctrl, bit2=alt, bit3=meta
    pub flags: u32,
    /// Event-specific payload (union-style usage)
    /// - KeyEventData for KeyDown/KeyUp
    /// - TextInputData for TextInput/ImeComposition/ImeCommit
    pub payload: [u8; 24],
}

impl RawInputEvent {
    /// Get event type as enum
    pub fn get_event_type(&self) -> Option<InputEventType> {
        InputEventType::from_u8(self.event_type)
    }

    /// Get payload as KeyEventData (for KeyDown/KeyUp events)
    pub fn as_key_event(&self) -> Option<KeyEventData> {
        if matches!(self.event_type, 5 | 6) {
            // KeyDown = 5, KeyUp = 6
            unsafe { Some(std::ptr::read(self.payload.as_ptr() as *const _)) }
        } else {
            None
        }
    }

    /// Get payload as TextInputData (for TextInput/ImeComposition/ImeCommit events)
    pub fn as_text_input(&self) -> Option<TextInputData> {
        if matches!(self.event_type, 7 | 8 | 9) {
            // TextInput = 7, ImeComposition = 8, ImeCommit = 9
            unsafe { Some(std::ptr::read(self.payload.as_ptr() as *const _)) }
        } else {
            None
        }
    }

    /// Create a new keyboard event
    pub fn new_key_event(
        event_type: InputEventType,
        key_code: u32,
        char_code: u32,
        modifiers: u8,
        repeat_count: u8,
    ) -> Self {
        let key_data = KeyEventData {
            key_code,
            char_code,
            modifiers,
            repeat_count,
            _padding: [0; 2],
        };
        // Copy KeyEventData into payload (KeyEventData is 12 bytes, payload is 24)
        let mut payload = [0u8; 24];
        unsafe {
            std::ptr::copy_nonoverlapping(
                &key_data as *const _ as *const u8,
                payload.as_mut_ptr(),
                std::mem::size_of::<KeyEventData>(),
            );
        }
        Self {
            timestamp: 0,
            pointer_id: 0,
            event_type: event_type as u8,
            _padding: [0; 3],
            x: key_code as f32,
            y: char_code as f32,
            pressure: modifiers as f32,
            delta_x: 0.0,
            delta_y: 0.0,
            target_node_id: 0,
            flags: modifiers as u32,
            payload,
        }
    }

    /// Create a new text input event
    pub fn new_text_input(
        event_type: InputEventType,
        text: &str,
        cursor_pos: u8,
    ) -> Option<Self> {
        let len = text.len();
        if len > 16 {
            return None; // Text too long for inline storage
        }
        let mut text_bytes = [0u8; 16];
        text_bytes[..len].copy_from_slice(text.as_bytes());
        let text_data = TextInputData {
            len: len as u8,
            text: text_bytes,
            cursor_pos,
            _padding: [0; 6],
        };
        // Copy TextInputData into payload (TextInputData is 24 bytes)
        let mut payload = [0u8; 24];
        unsafe {
            std::ptr::copy_nonoverlapping(
                &text_data as *const _ as *const u8,
                payload.as_mut_ptr(),
                std::mem::size_of::<TextInputData>(),
            );
        }
        Some(Self {
            timestamp: 0,
            pointer_id: 0,
            event_type: event_type as u8,
            _padding: [0; 3],
            x: 0.0,
            y: 0.0,
            pressure: 0.0,
            delta_x: 0.0,
            delta_y: 0.0,
            target_node_id: 0,
            flags: 0,
            payload,
        })
    }
}

impl Default for RawInputEvent {
    fn default() -> Self {
        Self {
            timestamp: 0,
            pointer_id: 0,
            event_type: InputEventType::PointerDown as u8,
            _padding: [0; 3],
            x: 0.0,
            y: 0.0,
            pressure: 1.0,
            delta_x: 0.0,
            delta_y: 0.0,
            target_node_id: 0,
            flags: 0,
            payload: [0; 24],
        }
    }
}

/// Buffer capacity: 100 events (~3.2KB)
/// 
/// At 120Hz sampling rate can cache ~830ms of events,
/// sufficient for frame rate fluctuations
pub const INPUT_BUFFER_CAPACITY: usize = 100;

/// Input event ring buffer
/// 
/// Single-producer single-consumer model:
/// - Producer: Host-side input thread
/// - Consumer: WASM logic thread
/// 
/// Uses wrapping_add for lock-free ring buffer
#[repr(C)]
pub struct InputBuffer {
    /// Write position (host-side monotonic increment)
    pub write_idx: u32,
    /// Read position (WASM-side monotonic increment)
    pub read_idx: u32,
    /// Overflow count (for debugging)
    pub overflow_count: u32,
    /// Reserved (for future use like last overflow time)
    _reserved: u32,
    /// Event storage array
    pub events: [RawInputEvent; INPUT_BUFFER_CAPACITY],
}

impl InputBuffer {
    /// Create empty input buffer
    pub const fn new() -> Self {
        Self {
            write_idx: 0,
            read_idx: 0,
            overflow_count: 0,
            _reserved: 0,
            events: [RawInputEvent {
                timestamp: 0,
                pointer_id: 0,
                event_type: InputEventType::PointerDown as u8,
                _padding: [0; 3],
                x: 0.0,
                y: 0.0,
                pressure: 1.0,
                delta_x: 0.0,
                delta_y: 0.0,
                target_node_id: 0,
                flags: 0,
                payload: [0; 24],
            }; INPUT_BUFFER_CAPACITY],
        }
    }

    /// Check if buffer is empty
    pub fn is_empty(&self) -> bool {
        self.read_idx == self.write_idx
    }

    /// Check if buffer is full
    pub fn is_full(&self) -> bool {
        self.write_idx - self.read_idx >= INPUT_BUFFER_CAPACITY as u32
    }

    /// Current event count
    pub fn len(&self) -> usize {
        (self.write_idx - self.read_idx) as usize
    }

    /// Push event (called by host)
    /// 
    /// Returns true on success, false if buffer full (event dropped)
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

    /// Pop event (called by WASM)
    /// 
    /// Returns None if buffer empty
    pub fn pop(&mut self) -> Option<RawInputEvent> {
        if self.is_empty() {
            return None;
        }
        let idx = (self.read_idx % INPUT_BUFFER_CAPACITY as u32) as usize;
        let event = self.events[idx];
        self.read_idx += 1;
        Some(event)
    }

    /// Batch read all available events
    /// 
    /// Used to process all accumulated events at frame start
    pub fn drain(&mut self) -> InputBufferDrainIterator<'_> {
        InputBufferDrainIterator { buffer: self }
    }

    /// Peek next event (without popping)
    pub fn peek(&self) -> Option<&RawInputEvent> {
        if self.is_empty() {
            return None;
        }
        let idx = (self.read_idx % INPUT_BUFFER_CAPACITY as u32) as usize;
        Some(&self.events[idx])
    }

    /// Clear buffer
    pub fn clear(&mut self) {
        self.read_idx = self.write_idx;
    }

    /// Get and reset overflow count
    pub fn take_overflow_count(&mut self) -> u32 {
        let count = self.overflow_count;
        self.overflow_count = 0;
        count
    }
}

/// Input buffer batch read iterator
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

/// Input event flag constants
pub mod input_flags {
    /// From stylus
    pub const STYLUS: u32 = 1 << 0;
    /// From eraser (stylus flipped)
    pub const ERASER: u32 = 1 << 1;
    /// Primary button (left mouse button / main finger)
    pub const PRIMARY: u32 = 1 << 2;
    /// Secondary button (right mouse button / secondary finger)
    pub const SECONDARY: u32 = 1 << 3;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_buffer_basic() {
        let mut buffer = InputBuffer::new();
        
        // Initial state
        assert!(buffer.is_empty());
        assert!(!buffer.is_full());
        assert_eq!(buffer.len(), 0);
        
        // Push event
        let event = RawInputEvent::default();
        assert!(buffer.push(event));
        assert!(!buffer.is_empty());
        assert_eq!(buffer.len(), 1);
        
        // Pop event
        let popped = buffer.pop();
        assert!(popped.is_some());
        assert!(buffer.is_empty());
    }

    #[test]
    fn test_input_buffer_wrap_around() {
        let mut buffer = InputBuffer::new();
        
        // Write 100 events
        for i in 0..INPUT_BUFFER_CAPACITY {
            let mut event = RawInputEvent::default();
            event.pointer_id = i as u32;
            assert!(buffer.push(event));
        }
        
        assert!(buffer.is_full());
        assert_eq!(buffer.len(), INPUT_BUFFER_CAPACITY);
        
        // Read 50
        for _ in 0..50 {
            buffer.pop();
        }
        
        assert_eq!(buffer.len(), INPUT_BUFFER_CAPACITY - 50);
        
        // Write 50 more (test wrap-around)
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
        
        // Fill buffer
        for _ in 0..INPUT_BUFFER_CAPACITY {
            assert!(buffer.push(RawInputEvent::default()));
        }
        
        // Next should fail
        assert!(!buffer.push(RawInputEvent::default()));
        assert_eq!(buffer.overflow_count, 1);
        
        // Try again
        assert!(!buffer.push(RawInputEvent::default()));
        assert_eq!(buffer.overflow_count, 2);
    }

    #[test]
    fn test_input_buffer_drain() {
        let mut buffer = InputBuffer::new();
        
        // Write 10 events
        for i in 0..10 {
            let mut event = RawInputEvent::default();
            event.pointer_id = i as u32;
            buffer.push(event);
        }
        
        // Batch read
        let events: Vec<_> = buffer.drain().collect();
        assert_eq!(events.len(), 10);
        assert!(buffer.is_empty());
        
        // Verify order
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
        assert_eq!(InputEventType::from_u8(5), Some(InputEventType::KeyDown));
        assert_eq!(InputEventType::from_u8(6), Some(InputEventType::KeyUp));
        assert_eq!(InputEventType::from_u8(7), Some(InputEventType::TextInput));
        assert_eq!(InputEventType::from_u8(8), Some(InputEventType::ImeComposition));
        assert_eq!(InputEventType::from_u8(9), Some(InputEventType::ImeCommit));
        assert_eq!(InputEventType::from_u8(99), None);
    }

    #[test]
    fn test_key_event_data() {
        let event = RawInputEvent::new_key_event(InputEventType::KeyDown, 65, 97, 0b0001, 0);
        assert_eq!(event.event_type, 5);

        let key_data = event.as_key_event().unwrap();
        assert_eq!(key_data.key_code, 65);
        assert_eq!(key_data.char_code, 97);
        assert_eq!(key_data.modifiers, 0b0001);
        assert_eq!(key_data.repeat_count, 0);
    }

    #[test]
    fn test_text_input_data() {
        let event = RawInputEvent::new_text_input(InputEventType::TextInput, "hello", 5).unwrap();
        assert_eq!(event.event_type, 7);

        let text_data = event.as_text_input().unwrap();
        assert_eq!(text_data.len, 5);
        assert_eq!(&text_data.text[..5], b"hello");
        assert_eq!(text_data.cursor_pos, 5);
    }

    #[test]
    fn test_text_input_too_long() {
        // Text longer than 16 bytes should be rejected
        let result = RawInputEvent::new_text_input(InputEventType::TextInput, "this is a very long text", 0);
        assert!(result.is_none());
    }

    #[test]
    fn test_raw_input_event_size() {
        // Verify the struct size is 72 bytes for predictable memory layout
        // 8+4+1+3+4+4+4+4+4+4+4+24 = 72
        assert_eq!(std::mem::size_of::<RawInputEvent>(), 72);
    }
}
