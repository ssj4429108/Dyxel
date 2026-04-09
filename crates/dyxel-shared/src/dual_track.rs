// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Dual-Track Memory Architecture
//!
//! Split shared memory into:
//! - Registry (Static): Node structure, append-only
//! - CommandStream (Dynamic): Property changes, ring buffer with backpressure

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU32, Ordering};

// ============================================================================
// Constants
// ============================================================================

/// Registry magic: "REGS"
pub const REGISTRY_MAGIC: u32 = 0x52454753;

/// CommandStream magic: "CMDS"
pub const COMMAND_STREAM_MAGIC: u32 = 0x434D4453;

/// Sentinel value for boundary checking
pub const SENTINEL_MAGIC: u32 = 0xDEADBEEF;

/// Registry capacity: ~800 nodes in 32KB
pub const REGISTRY_CAPACITY: usize = 800;

/// Registry size: 32KB
pub const REGISTRY_SIZE: usize = 32 * 1024;

/// CommandStream size: 96KB
pub const COMMAND_STREAM_SIZE: usize = 96 * 1024;

/// Watermark threshold: 80%
pub const WATERMARK_THRESHOLD: u32 = (COMMAND_STREAM_SIZE as u32 * 80) / 100;

/// Critical threshold: 95%
pub const CRITICAL_THRESHOLD: u32 = (COMMAND_STREAM_SIZE as u32 * 95) / 100;

/// Safety margin for NOP padding
pub const SAFETY_MARGIN: usize = 16;

/// NOP opcode
pub const OP_NOP: u8 = 0x00;

/// BulkCreateNode opcode
pub const OP_BULK_CREATE: u8 = 0x01;

/// SetColor opcode
pub const OP_SET_COLOR: u8 = 0x02;

/// SetTransform opcode
pub const OP_SET_TRANSFORM: u8 = 0x03;

// ============================================================================
// Registry (Static Area)
// ============================================================================

/// Node type enumeration
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeType {
    Container = 0,
    Text = 1,
    Button = 2,
    Image = 3,
}

/// Initialization mask bits
#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitMask {
    HasWidth = 1 << 0,
    HasHeight = 1 << 1,
    HasColor = 1 << 2,
    HasFlexDirection = 1 << 3,
    HasJustifyContent = 1 << 4,
    HasAlignItems = 1 << 5,
    HasFlexWrap = 1 << 6,
    HasMargin = 1 << 7,
    HasPadding = 1 << 8,
    HasBorderRadius = 1 << 9,
}

/// Compact registry node entry (16 bytes)
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RegistryNode {
    /// Node ID
    pub id: u32,
    /// Parent ID (u32::MAX for root)
    pub parent_id: u32,
    /// Node type
    pub node_type: u8,
    /// Which properties are initialized
    pub init_mask: u8,
    /// Node flags
    pub flags: u16,
    /// Index into style pool
    pub style_idx: u16,
    /// Reserved for alignment
    pub _reserved: u16,
}

/// Registry flags
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryFlags {
    None = 0,
    Initialized = 1 << 0,
    Frozen = 1 << 1, // No more nodes can be added
    Dirty = 1 << 2,  // Host needs to resync
}

/// Registry header (64 bytes)
#[repr(C, align(64))]
pub struct RegistryHeader {
    /// Magic: 0x52454753
    pub magic: u32,
    /// Version counter (incremented on each change)
    pub version: AtomicU32,
    /// Current node count
    pub node_count: AtomicU32,
    /// Maximum capacity
    pub capacity: u32,
    /// Flags (use AtomicU32 for thread-safe access)
    pub flags: AtomicU32,
    /// _padding to 64 bytes
    pub _padding: [u32; 9],
}

/// Registry area (32KB total)
///
/// Layout: Header (64B) + Nodes (~12.5KB) + Padding + Sentinel (4B)
#[repr(C, align(4096))]
pub struct Registry {
    pub header: RegistryHeader,
    /// Node entries (fixed array)
    pub nodes: [RegistryNode; REGISTRY_CAPACITY],
    /// Padding to align sentinel
    pub _padding: [u8; REGISTRY_SIZE - 64 - (REGISTRY_CAPACITY * 16) - 4],
    /// Boundary sentinel
    pub sentinel: u32,
}

impl Registry {
    /// Create new empty registry
    pub const fn new() -> Self {
        Self {
            header: RegistryHeader {
                magic: 0,
                version: AtomicU32::new(0),
                node_count: AtomicU32::new(0),
                capacity: REGISTRY_CAPACITY as u32,
                flags: AtomicU32::new(0),
                _padding: [0; 9],
            },
            nodes: [RegistryNode {
                id: u32::MAX,
                parent_id: u32::MAX,
                node_type: 0,
                init_mask: 0,
                flags: 0,
                style_idx: 0,
                _reserved: 0,
            }; REGISTRY_CAPACITY],
            _padding: [0; REGISTRY_SIZE - 64 - (REGISTRY_CAPACITY * 16) - 4],
            sentinel: 0,
        }
    }

    /// Initialize registry
    pub fn initialize(&mut self) {
        self.header.magic = REGISTRY_MAGIC;
        self.header.version.store(1, Ordering::Release);
        self.header.node_count.store(0, Ordering::Release);
        self.header
            .flags
            .store(RegistryFlags::Initialized as u32, Ordering::Release);
        self.sentinel = SENTINEL_MAGIC;
    }

    /// Check if registry is valid
    pub fn is_valid(&self) -> bool {
        self.header.magic == REGISTRY_MAGIC
            && self.sentinel == SENTINEL_MAGIC
            && self.header.flags.load(Ordering::Acquire) & RegistryFlags::Initialized as u32 != 0
    }

    /// Check if sentinel is intact (no overflow)
    pub fn check_sentinel(&self) -> bool {
        self.sentinel == SENTINEL_MAGIC
    }

    /// Add a node to registry
    ///
    /// SAFETY: Must be called from WASM side with exclusive access
    pub unsafe fn add_node(&self, node: RegistryNode) -> Option<u32> {
        let count = self.header.node_count.load(Ordering::Acquire);
        if count >= self.header.capacity {
            return None;
        }

        // Write node
        let ptr = self.nodes.as_ptr().add(count as usize) as *mut RegistryNode;
        ptr.write(node);

        // Increment count and version
        self.header.node_count.store(count + 1, Ordering::Release);
        self.header.version.fetch_add(1, Ordering::Release);

        Some(count)
    }

    /// Get node at index
    pub fn get_node(&self, idx: usize) -> Option<&RegistryNode> {
        if idx < self.header.node_count.load(Ordering::Acquire) as usize {
            Some(&self.nodes[idx])
        } else {
            None
        }
    }

    /// Get current version
    pub fn version(&self) -> u32 {
        self.header.version.load(Ordering::Acquire)
    }

    /// Get node count
    pub fn node_count(&self) -> u32 {
        self.header.node_count.load(Ordering::Acquire)
    }

    /// Mark as dirty (Host needs to resync)
    pub fn mark_dirty(&self) {
        let old = self.header.flags.load(Ordering::Relaxed);
        self.header
            .flags
            .store(old | RegistryFlags::Dirty as u32, Ordering::Release);
    }

    /// Clear dirty flag
    pub fn clear_dirty(&self) {
        let old = self.header.flags.load(Ordering::Relaxed);
        self.header
            .flags
            .store(old & !(RegistryFlags::Dirty as u32), Ordering::Release);
    }
}

// SAFETY: Registry is designed for single-writer (WASM), single-reader (Host)
unsafe impl Sync for Registry {}
unsafe impl Send for Registry {}

// ============================================================================
// Command Stream (Dynamic Area)
// ============================================================================

/// Throttle levels
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ThrottleLevel {
    /// < 50% usage
    Normal = 0,
    /// 50-80% usage
    Elevated = 1,
    /// 80-95% usage
    Warning = 2,
    /// > 95% usage
    Critical = 3,
}

impl ThrottleLevel {
    /// Calculate from usage percentage
    pub fn from_percent(percent: f32) -> Self {
        match percent {
            p if p < 50.0 => Self::Normal,
            p if p < 80.0 => Self::Elevated,
            p if p < 95.0 => Self::Warning,
            _ => Self::Critical,
        }
    }
}

/// Command stream header (64 bytes)
#[repr(C, align(64))]
pub struct CommandStreamHeader {
    /// Magic: 0x434D4453
    pub magic: u32,
    /// Transaction ticket (version counter)
    pub ticket: AtomicU32,
    /// Write head (WASM)
    pub write_head: AtomicU32,
    /// Read tail (Host)
    pub read_tail: AtomicU32,
    /// Watermark threshold
    pub watermark: u32,
    /// Critical threshold
    pub critical: u32,
    /// Capacity
    pub capacity: u32,
    /// Flags
    pub flags: u32,
    /// _padding to 64 bytes
    pub _padding: [u32; 4],
}

/// Command Stream (96KB total)
///
/// Layout: Header (64B) + Data (~95.9KB) + Sentinel (4B)
#[repr(C, align(4096))]
pub struct CommandStream {
    pub header: CommandStreamHeader,
    /// Ring buffer data
    pub data: [UnsafeCell<u8>; COMMAND_STREAM_SIZE - 64 - 4],
    /// Boundary sentinel
    pub sentinel: u32,
}

impl CommandStream {
    /// Create new command stream
    pub const fn new() -> Self {
        const DEFAULT_CELL: UnsafeCell<u8> = UnsafeCell::new(0);
        Self {
            header: CommandStreamHeader {
                magic: 0,
                ticket: AtomicU32::new(0),
                write_head: AtomicU32::new(0),
                read_tail: AtomicU32::new(0),
                watermark: WATERMARK_THRESHOLD,
                critical: CRITICAL_THRESHOLD,
                capacity: (COMMAND_STREAM_SIZE - 64 - 4) as u32,
                flags: 0,
                _padding: [0; 4],
            },
            data: [DEFAULT_CELL; COMMAND_STREAM_SIZE - 64 - 4],
            sentinel: 0,
        }
    }

    /// Initialize command stream
    pub fn initialize(&mut self) {
        self.header.magic = COMMAND_STREAM_MAGIC;
        self.header.ticket.store(1, Ordering::Release);
        self.header.write_head.store(0, Ordering::Release);
        self.header.read_tail.store(0, Ordering::Release);
        self.sentinel = SENTINEL_MAGIC;
    }

    /// Check if valid
    pub fn is_valid(&self) -> bool {
        self.header.magic == COMMAND_STREAM_MAGIC && self.sentinel == SENTINEL_MAGIC
    }

    /// Check sentinel
    pub fn check_sentinel(&self) -> bool {
        self.sentinel == SENTINEL_MAGIC
    }

    /// Calculate used space
    pub fn used_space(&self) -> u32 {
        let head = self.header.write_head.load(Ordering::Acquire);
        let tail = self.header.read_tail.load(Ordering::Acquire);

        if head >= tail {
            head - tail
        } else {
            self.header.capacity - tail + head
        }
    }

    /// Calculate free space
    pub fn free_space(&self) -> u32 {
        self.header.capacity - self.used_space()
    }

    /// Get usage percentage
    pub fn usage_percent(&self) -> f32 {
        (self.used_space() as f32 / self.header.capacity as f32) * 100.0
    }

    /// Get current throttle level
    pub fn throttle_level(&self) -> ThrottleLevel {
        ThrottleLevel::from_percent(self.usage_percent())
    }

    /// Check if watermark exceeded
    pub fn is_watermark_exceeded(&self) -> bool {
        self.used_space() >= self.header.watermark
    }

    /// Check if critical
    pub fn is_critical(&self) -> bool {
        self.used_space() >= self.header.critical
    }

    /// Reserve space (with NOP padding if needed)
    ///
    /// SAFETY: Single writer (WASM)
    pub unsafe fn reserve_space(&self, size: usize) -> Option<usize> {
        if size > self.header.capacity as usize - SAFETY_MARGIN {
            return None; // Too large
        }

        let head = self.header.write_head.load(Ordering::Relaxed) as usize;
        let tail = self.header.read_tail.load(Ordering::Acquire) as usize;
        let capacity = self.header.capacity as usize;

        // Calculate free space considering wrap-around
        let free = if head >= tail {
            capacity - (head - tail)
        } else {
            tail - head
        };

        if free < size + SAFETY_MARGIN {
            return None; // Not enough space
        }

        // Check if we need to wrap around
        let space_to_end = capacity - head;
        if space_to_end < size {
            // Need to wrap - fill rest with NOP
            for i in 0..space_to_end {
                *self.data[head + i].get() = OP_NOP;
            }
            // Return position at start
            Some(0)
        } else {
            Some(head)
        }
    }

    /// Write command to stream
    ///
    /// SAFETY: Single writer (WASM), must call reserve_space first
    pub unsafe fn write_command(&self, opcode: u8, data: &[u8], pos: usize) -> bool {
        let total_len = 1 + data.len();
        let capacity = self.header.capacity as usize;

        // Write opcode
        *self.data[pos % capacity].get() = opcode;

        // Write data
        for (i, &byte) in data.iter().enumerate() {
            *self.data[(pos + 1 + i) % capacity].get() = byte;
        }

        // Update write head with release ordering
        let new_head = ((pos + total_len) % capacity) as u32;
        self.header.write_head.store(new_head, Ordering::Release);

        true
    }

    /// Commit write (increment ticket)
    pub fn commit(&self) {
        core::sync::atomic::fence(Ordering::Release);
        self.header.ticket.fetch_add(1, Ordering::Release);
    }

    /// Get current ticket
    pub fn ticket(&self) -> u32 {
        self.header.ticket.load(Ordering::Acquire)
    }

    /// Read next command (Host side)
    pub fn read_command(&self) -> Option<(u8, usize)> {
        let head = self.header.write_head.load(Ordering::Acquire);
        let tail = self.header.read_tail.load(Ordering::Relaxed);

        if head == tail {
            return None; // Empty
        }

        let tail_usize = tail as usize;
        let capacity = self.header.capacity as usize;

        // Read opcode
        let opcode = unsafe { *self.data[tail_usize % capacity].get() };

        Some((opcode, tail_usize))
    }

    /// Advance read tail (Host side)
    pub fn advance(&self, bytes: u32) {
        let tail = self.header.read_tail.load(Ordering::Relaxed);
        let new_tail = ((tail + bytes) % self.header.capacity) as u32;
        self.header.read_tail.store(new_tail, Ordering::Release);
    }

    /// Reset stream
    pub fn reset(&self) {
        self.header.write_head.store(0, Ordering::Release);
        self.header.read_tail.store(0, Ordering::Release);
        self.header.ticket.fetch_add(1, Ordering::Relaxed);
    }
}

// SAFETY: CommandStream is designed for SPSC
unsafe impl Sync for CommandStream {}
unsafe impl Send for CommandStream {}

// ============================================================================
// Dual-Track Memory Layout
// ============================================================================

/// Complete dual-track memory layout (128KB)
#[repr(C, align(4096))]
pub struct DualTrackMemory {
    /// Registry area (32KB)
    pub registry: Registry,
    /// Command stream area (96KB)
    pub command_stream: CommandStream,
}

impl DualTrackMemory {
    /// Create new dual-track memory
    pub const fn new() -> Self {
        Self {
            registry: Registry::new(),
            command_stream: CommandStream::new(),
        }
    }

    /// Initialize both areas
    pub fn initialize(&mut self) {
        self.registry.initialize();
        self.command_stream.initialize();
    }

    /// Validate entire memory layout
    pub fn is_valid(&self) -> bool {
        self.registry.is_valid()
            && self.command_stream.is_valid()
            && self.registry.check_sentinel()
            && self.command_stream.check_sentinel()
    }

    /// Check all sentinels
    pub fn check_sentinels(&self) -> bool {
        self.registry.check_sentinel() && self.command_stream.check_sentinel()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_basic() {
        let mut reg = Registry::new();
        reg.initialize();

        assert!(reg.is_valid());
        assert_eq!(reg.node_count(), 0);

        // Add node
        let node = RegistryNode {
            id: 0,
            parent_id: u32::MAX,
            node_type: NodeType::Container as u8,
            init_mask: InitMask::HasColor as u8,
            flags: 0,
            style_idx: 0,
            _reserved: 0,
        };

        unsafe {
            assert!(reg.add_node(node).is_some());
        }

        assert_eq!(reg.node_count(), 1);
        assert_eq!(reg.version(), 2);

        // Read back
        let retrieved = reg.get_node(0).unwrap();
        assert_eq!(retrieved.id, 0);
        assert_eq!(retrieved.node_type, NodeType::Container as u8);
    }

    #[test]
    fn test_command_stream_basic() {
        let mut stream = CommandStream::new();
        stream.initialize();

        assert!(stream.is_valid());
        assert_eq!(stream.usage_percent(), 0.0);

        // Write command
        let data = [1u8, 2, 3, 4];
        unsafe {
            if let Some(pos) = stream.reserve_space(5) {
                assert!(stream.write_command(OP_SET_COLOR, &data, pos));
            }
        }

        assert!(stream.used_space() > 0);
        stream.commit();

        // Read command
        if let Some((opcode, _pos)) = stream.read_command() {
            assert_eq!(opcode, OP_SET_COLOR);
        } else {
            panic!("Should have command");
        }
    }

    #[test]
    fn test_throttle_levels() {
        assert_eq!(ThrottleLevel::from_percent(30.0), ThrottleLevel::Normal);
        assert_eq!(ThrottleLevel::from_percent(60.0), ThrottleLevel::Elevated);
        assert_eq!(ThrottleLevel::from_percent(85.0), ThrottleLevel::Warning);
        assert_eq!(ThrottleLevel::from_percent(97.0), ThrottleLevel::Critical);
    }

    #[test]
    fn test_dual_track_memory() {
        let mut mem = DualTrackMemory::new();
        mem.initialize();

        assert!(mem.is_valid());
        assert!(mem.check_sentinels());
    }
}
