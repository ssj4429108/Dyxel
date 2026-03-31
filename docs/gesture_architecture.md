# Dyxel Gesture System Architecture (Optimized for 1000+ Nodes)

## Problem Statement

Current O(N) hit testing doesn't scale to thousands of nodes:
- Every touch requires scanning all nodes
- Event bubbling requires parent lookups
- Dual maintenance of tree structure (Host + WASM)

## Proposed Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                         Host Side                            │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────┐  │
│  │ Touch Input  │→ │ SpatialIndex │→ │ GestureArena     │  │
│  └──────────────┘  │ (O(log N))   │  │ (Routes Events)  │  │
│                    └──────────────┘  └──────────────────┘  │
│                              ↓                               │
│                    ┌──────────────────┐                     │
│                    │ HandlerRegistry  │                     │
│                    │ (Knows which     │                     │
│                    │  nodes have      │                     │
│                    │  gesture handlers)│                    │
│                    └──────────────────┘                     │
└─────────────────────────────────────────────────────────────┘
                              ↓
                    ┌──────────────────┐
                    │  Direct WASM     │
                    │  Handler Call    │
                    │  (No Bubbling)   │
                    └──────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────┐
│                        WASM Side                             │
│  ┌──────────────────────────────────────────────────────┐  │
│  │           Simple Handler Dispatch                     │  │
│  │  No PARENT_MAP needed, no tree traversal             │  │
│  │  Host tells WASM exactly which node to call          │  │
│  └──────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

## Key Changes

### 1. Host-Side Handler Registry

```rust
// Host tracks which nodes have which handlers
struct HandlerRegistry {
    tap_handlers: HashSet<u32>,
    long_press_handlers: HashSet<u32>,
    pan_handlers: HashSet<u32>,
}

impl HandlerRegistry {
    fn find_handler(&self, mut path: &[u32], handler_type: HandlerType) -> Option<u32> {
        // Walk bubble path, find first node with handler
        for &node_id in path {
            if self.has_handler(node_id, handler_type) {
                return Some(node_id);
            }
        }
        None
    }
}
```

### 2. Spatial Index for O(log N) Hit Testing

```rust
pub struct SpatialIndex {
    // Grid-based spatial hashing
    grid: HashMap<(i32, i32), Vec<u32>>,
    // Or R-tree for better distribution
    rtree: RTree<Rect>,
}
```

### 3. Direct Handler Invocation

Instead of sending `GestureTap` command and letting WASM bubble:

```rust
// Host finds the right handler and calls it directly
let target = registry.find_handler(&bubble_path, HandlerType::Tap);
if let Some(node_id) = target {
    wasm.call_handler(node_id, GestureEvent::Tap { x, y });
}
```

WASM exports simple functions:
```rust
#[no_mangle]
pub extern "C" fn on_tap_handler(node_id: u32, x: f32, y: f32) {
    // Direct dispatch, no bubbling logic
}
```

## Benefits

1. **O(log N) Hit Testing** - Spatial index scales to thousands of nodes
2. **No Dual Tree** - Host maintains single source of truth
3. **No Event Bubbling in WASM** - Host decides target, simpler WASM code
4. **Better Caching** - Spatial index can persist across frames

## Migration Path

Phase 1: Add spatial index (keep current bubbling)
Phase 2: Add handler registry, test direct invocation
Phase 3: Remove PARENT_MAP, simplify WASM
