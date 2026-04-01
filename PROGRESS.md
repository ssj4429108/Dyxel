# Dyxel UI Framework - Progress Report

**Date**: 2026-04-01

## ✅ Completed Features

### 1. Generational ID System (代际ID系统)
- **Location**: `crates/dyxel-shared/src/state.rs`
- **Feature**: NodeHandle with slot+generation pattern
- **Benefits**: Prevents stale ID issues, safe node reuse
- **Tests**: 27 unit tests in `state_tests.rs`

### 2. Dynamic Capacity Expansion (动态容量扩容)
- **Location**: `crates/dyxel-shared/src/state.rs`
- **Levels**: [256, 512, 1024, 2048, 4096]
- **API**: `expand_capacity()`, `should_pre_expand()`

### 3. StressTest Sample
- **Location**: `sample/src/stress_test.rs`
- **Features**:
  - Batch add/remove/clear nodes
  - Capacity usage visualization
  - Expansion event logging
  - All text colors changed to black

### 4. Gesture System Core (手势系统核心)

#### 4.1 Basic Gestures (基础手势)
- **RSX DSL**: `onTap`, `onDoubleTap`, `onLongPress`, `onPanUpdate`
- **Working**: Event detection and dispatch

#### 4.2 Gesture Arena (手势竞技场)
- **Location**: `crates/dyxel-gesture/src/`
- **Feature**: Competing gesture resolution
- **Example**: Tap vs LongPress competition

#### 4.3 Composite Gestures (复合手势)
- **RSX DSL**: `gesture:` attribute
- **Types**: 
  - `ExclusiveGesture` (互斥手势)
  - `SequenceGesture` (顺序手势)
  - `ParallelGesture` (并行使势)
- **Implementation**: `GestureConfig::apply_to()` in `dyxel-view/src/lib.rs`

### 5. RGBA Color Support
- **Change**: `color: (u32, u32, u32)` → `(u32, u32, u32, u32)`
- **Updated**: All samples use RGBA format

### 6. Absolute Coordinate Fix (绝对坐标修复)
- **Issue**: `sync_layout_to_wasm` wrote relative coordinates
- **Fix**: BFS traversal to calculate absolute positions
- **Impact**: Hit testing now works correctly for nested nodes

### 7. Spatial Hit Tester (空间命中测试)
- **Feature**: O(1) grid-based hit testing
- **File**: `crates/dyxel-gesture/src/spatial_hit_tester.rs`

## 🚧 TODO / Known Issues

### High Priority

#### 1. State Dynamic Binding Not Working
- **Issue**: `width: {dynamic_size.get()}` doesn't update on state change
- **Location**: `crates/dyxel-rsx/src/lib.rs`, `crates/dyxel-view/src/lib.rs`
- **Attempted**: 
  - RSX macro detection of `{state.get()}` pattern
  - Signal-to-Prop conversion with `.sig()`, `.sig_size()`, `.sig_color()`
- **Status**: Syntax compiles, but runtime updates not working
- **Possible Causes**:
  - Executor not polling signals correctly
  - Missing trigger for re-layout when properties change
  - Signal subscription not established properly

#### 2. Layout Invalidation on Property Change
- **Issue**: When State-bound properties change, layout doesn't recalculate
- **Need**: Dirty flag system for dynamic property changes

### Medium Priority

#### 3. Text State Binding
- **Issue**: `Text("{count}")` interpolation works but direct binding not implemented
- **Current**: Static evaluation at creation time

#### 4. Performance Optimization
- **Spatial Index**: Only syncs new nodes, doesn't update existing node layouts
- **Hit Testing**: Needs to handle layout changes dynamically

### Low Priority

#### 5. Composite Gesture Full Implementation
- **Current**: Registers handlers, but sequence/parallel logic in Host incomplete
- **Working**: ExclusiveGesture with Arena competition

## 📁 Key Files Modified

### Core Changes
```
crates/dyxel-core/src/bridge.rs          - build_bubble_path with parent chain
crates/dyxel-core/src/runtime.rs         - sync_layout_to_wasm with absolute coords
crates/dyxel-core/src/handler_registry.rs - Gesture handler tracking

crates/dyxel-gesture/src/spatial_hit_tester.rs - Spatial index for hit testing
crates/dyxel-gesture/src/lib.rs          - GestureRouter

crates/dyxel-shared/src/state.rs         - Generational ID + Dynamic capacity
crates/dyxel-shared/src/protocol.rs      - New opcodes for gestures

crates/dyxel-view/src/lib.rs             - BaseView trait + GestureConfig
crates/dyxel-view/src/gesture.rs         - Gesture types and registration

crates/dyxel-rsx/src/lib.rs              - RSX macro with gesture support
```

### Sample Changes
```
sample/src/lib.rs                        - Module exports
sample/src/stress_test.rs                - Stress test with black text
sample/src/gesture_orchestration.rs      - NEW: Gesture testing sample
```

## 🔄 Next Steps

1. **Fix State Dynamic Binding**
   - Debug signal polling in executor
   - Add layout invalidation on property change
   - Verify Mutable/Signal integration

2. **Complete Gesture System**
   - Host-side sequence tracking
   - Parallel gesture state machine

3. **Performance**
   - Spatial index incremental updates
   - Layout dirty tracking optimization

## 📝 Notes

- **Build**: `./build_android.sh` works for all targets
- **Test**: StressTest validates ID system and capacity
- **Gesture**: GestureOrchestration demonstrates all gesture features
- **Issue**: State binding needs executor/layout integration fix


• gesture_demo 中 tick.call() 卡住不返回
  • 可能是 WASM runtime 或 tick 函数本身的问题

  明天可以继续从这个点排查。需要检查 guest_tick/dyxel_view_tick 的执行流程，看看在哪里陷入了死循环或阻塞。