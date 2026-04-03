# Dyxel Gesture System Design

**Version**: 1.0  
**Status**: Implemented  
**Goal**: Flutter-compatible GestureArena architecture  
**Last Updated**: 2026-04-03

## Architecture

The gesture system is based on Flutter's GestureArena pattern with three layers:

```
┌─────────────────────────────────────────────────────────────┐
│ Layer 3: WASM Guest (Application)                           │
│  - Gesture handlers (onTap, onLongPress, onPan, etc.)       │
└────────────────────────────┬────────────────────────────────┘
                             │ WASM/Host Protocol
┌────────────────────────────┼────────────────────────────────┐
│ Layer 2: Host Arena        │                                │
│  - GestureArenaManager     │  - GestureArena per pointer    │
│  - Recognizers compete     │  - Exclusive/Simultaneous      │
└────────────────────────────┴────────────────────────────────┘
                             │
┌────────────────────────────┴────────────────────────────────┐
│ Layer 1: Platform                                           │
│  - Raw pointer events (Down/Move/Up/Cancel)                 │
│  - Multi-touch support                                      │
└─────────────────────────────────────────────────────────────┘
```

## Core Components

### GestureArenaManager
Manages all gesture arenas, one per active pointer. Routes pointer events to the appropriate arena.

### GestureArena
Holds competing recognizers for a single pointer sequence. Resolves competition when:
- A recognizer calls `accept()` - it wins, others are rejected
- All recognizers reach terminal state (Ended/Failed/Cancelled)

### RecognizerState
```rust
pub enum RecognizerState {
    Ready,      // Ready to start
    Possible,   // May be this gesture, waiting for more data
    Began,      // Gesture started (continuous)
    Changed,    // Gesture updated (continuous)
    Ended,      // Successfully completed
    Cancelled,  // Cancelled by system or other gesture
    Failed,     // Recognition failed
}
```

## Recognizers

### TapGestureRecognizer
- Configurable tap count (1=single, 2=double, etc.)
- Respects slop, multi-tap timeout, multi-tap slop
- Fires on reaching target tap count or on timeout with partial count

### LongPressGestureRecognizer
- Timer-based (default 500ms)
- Triggers `LongPressStart` on timer, `LongPressEnd` on pointer up
- Fails if pointer moves beyond slop

### PanGestureRecognizer
- Supports direction locking (Horizontal/Vertical/Any)
- Tracks velocity for momentum
- Multi-pointer support (centroid tracking)

### ScaleGestureRecognizer
- Two-finger pinch zoom
- Calculates scale ratio and focal point
- Can work simultaneously with Pan

## Gesture Composition

### ExclusiveGesture
Only one recognizer can win. When one accepts, others are rejected.
```rust
ExclusiveGesture::new(vec![
    TapGesture::single_tap(),
    TapGesture::double_tap(),
])
```

### SimultaneousGesture
Multiple recognizers can win together.
```rust
SimultaneousGesture::new(vec![
    PanGesture::new(),
    ScaleGesture::new(),
])
```

### SequencedGesture
Recognizers must complete in order.
```rust
SequencedGesture::new(vec![
    LongPressGesture::new(),
    PanGesture::new(),
])
```

## Key Parameters (Flutter-aligned)

| Parameter | Value | Description |
|-----------|-------|-------------|
| `tap_slop` | 18.0 dp | Max movement for tap |
| `double_tap_slop` | 100.0 dp | Max distance between taps |
| `double_tap_timeout` | 300 ms | Max time between taps |
| `long_press_timeout` | 500 ms | Time to trigger long press |
| `pan_slop` | 18.0 dp | Movement to start pan |

## File Structure

```
crates/dyxel-gesture/src/
├── lib.rs                    # Public exports
├── arena.rs                  # GestureArenaManager & GestureArena
├── recognizer.rs             # Recognizer trait + implementations
├── gesture_composition.rs    # Exclusive/Simultaneous/Sequenced
├── router_integration.rs     # Flutter-style GestureRouter
├── events.rs                 # PointerEvent, GestureEvent types
├── router.rs                 # PointerRouter for raw events
├── hit_test.rs               # Hit testing utilities
└── spatial_hit_tester.rs     # Spatial hit testing
```

## Usage in dyxel-core

```rust
// In bridge.rs - Initialize router
GESTURE_ROUTER.with(|router| {
    *router.borrow_mut() = Some(GestureRouter::new());
});

// Route pointer events
let events = router.route_pointer_event(&pointer_event);
for event in events {
    dispatch_gesture_event(event);
}
```

## References

- [Flutter GestureArena](https://github.com/flutter/flutter/blob/master/packages/flutter/lib/src/gestures/arena.dart)
- [Flutter GestureRecognizer](https://github.com/flutter/flutter/tree/master/packages/flutter/lib/src/gestures)
