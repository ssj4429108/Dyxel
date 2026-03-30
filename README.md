<p align="center">
  <img src="assets/logo.svg" width="120" alt="Dyxel Logo">
</p>

<h1 align="center">Dyxel</h1>

<p align="center">
  <b>A Cross-Platform Dynamic UI Framework with Rust & WebAssembly</b>
</p>

<p align="center">
  <a href="docs/zh-cn/README.md">🇨🇳 中文</a>
  ·
  <a href="#overview">Overview</a>
  ·
  <a href="#architecture">Architecture</a>
  ·
  <a href="#quick-start">Quick Start</a>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0%20OR%20MIT-blue.svg" alt="License"></a>
  <img src="https://img.shields.io/badge/platform-iOS%20%7C%20Android%20%7C%20macOS%20%7C%20Web-orange.svg" alt="Platform">
  <img src="https://img.shields.io/badge/renderer-Vello%20%7C%20Impeller-purple.svg" alt="Renderer">
</p>

---

## Overview

**Dyxel** is a high-performance, cross-platform UI framework designed for building dynamic, interactive applications. It leverages Rust's safety guarantees and WebAssembly's portability to deliver near-native performance while enabling dynamic code delivery.

### Core Philosophy

- **Host + Guest Architecture**: Separate rendering engine (Host) from business logic (Guest) for maximum flexibility
- **Zero-Cost Abstractions**: Rust-powered performance without runtime overhead
- **True Dynamic Updates**: Deploy new UI logic via WASM without app store updates

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        Application Layer                         │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐             │
│  │   iOS App   │  │ Android App │  │   Web App   │  ┌────────┐ │
│  │   (Swift)   │  │  (Kotlin)   │  │  (JS/WASM)  │  │ macOS  │ │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘  └────────┘ │
└─────────┼────────────────┼────────────────┼────────────────────┘
          │                │                │
          └────────────────┴────────────────┘
                             │
                    ┌────────▼────────┐
                    │   Dyxel Host    │
                    │  (Rust Core)    │
                    │  • Rendering    │
                    │  • Layout       │
                    │  • Input        │
                    └────────┬────────┘
                             │
                    ┌────────▼────────┐
                    │  Shared Memory  │
                    │  Command Buffer │
                    └────────┬────────┘
                             │
                    ┌────────▼────────┐
                    │   Dyxel View    │
                    │  (WASM Guest)   │
                    │  • UI Logic     │
                    │  • Animations   │
                    │  • Events       │
                    └─────────────────┘
```

### Key Components

| Component | Description |
|-----------|-------------|
| **dyxel-core** | Host engine with platform abstraction, rendering coordination, and WASM runtime |
| **dyxel-render-api** | Abstract render backend interface |
| **dyxel-render-vello** | Vello-based GPU renderer (wgpu) |
| **dyxel-render-impeller** | Impeller-based renderer (experimental) |
| **dyxel-shared** | Shared types, protocol definitions, and command structures |
| **dyxel-view** | Guest-side UI framework with reactive signals and Shadow Layout |

---

## Dynamic Capabilities

### 1. Hot Update without App Store

Business logic compiles to WebAssembly and can be updated dynamically:

```rust
// sample/src/lib.rs - Guest-side UI code
#[no_mangle]
pub extern "C" fn guest_tick() {
    let f = FRAME_COUNT.fetch_add(1, Ordering::SeqCst) as f32;
    
    for i in 1..101 {
        let idx = i as f32;
        // Dynamic positioning based on time
        let x = 50.0 + (f * 0.03 + idx * 0.5).cos() * 40.0;
        let y = 50.0 + (f * 0.02 + idx * 0.3).sin() * 40.0;
        
        View { id: i }
            .inset((y, 0.0, 0.0, x))
            .color((
                (128.0 + (f * 0.02 + idx).cos() * 127.0) as u32,
                (128.0 + (f * 0.03 + idx * 0.5).sin() * 127.0) as u32,
                (128.0 + (idx * 2.0).cos() * 127.0) as u32
            ));
    }
    
    dyxel_view_tick();
}
```

### 2. Shared Memory Communication

Zero-copy communication between Host and Guest via shared buffer:

```rust
// Shared buffer layout
pub struct SharedBuffer {
    pub command_len: u32,
    pub max_node_id: u32,
    pub command_data: [u8; MAX_COMMAND_BYTES],  // Commands from Guest
    pub layout_results: [LayoutResult; MAX_NODES], // Layout results to Guest
    pub dirty_mask: [u8; 32],  // Dirty tracking
}
```

### 3. Shadow Layout (Zero-Latency)

WASM-side layout estimation eliminates frame delays:

```rust
// Instant layout query (0ms latency)
let layout = get_layout_estimated(view.id);
println!("Position: ({}, {})", layout.x, layout.y);

// Batch queries for hit testing
let hit_nodes = find_nodes_at_point(mouse_x, mouse_y, &candidates);
```

See [docs/shadow-layout.md](docs/shadow-layout.md) for details.

### 4. Cross-Platform Consistency

Same business logic runs on all platforms:

| Platform | Host Implementation | Guest Support |
|----------|---------------------|---------------|
| iOS | UniFFI + Swift bindings | ✅ |
| Android | JNI + Kotlin bindings | ✅ |
| macOS | Native window + winit | ✅ |
| Web | WASM-bindgen + Canvas | ✅ |

---

## Features

### Rendering

- **Vello Backend**: GPU-accelerated 2D vector graphics via wgpu
- **Impeller Backend** (experimental): Flutter's rendering engine ported to Rust
- **SPIR-V Shader Caching**: Precompiled shaders for faster Android startup

### Layout

- **Flexbox Layout**: Powered by [Taffy](https://github.com/DioxusLabs/taffy)
- **Shadow Layout**: WASM-side layout estimation for zero-latency queries (<16ms)
- **Responsive Design**: Percentage-based and absolute positioning
- **Border Radius**: Rounded rectangle support

### Reactivity

- **Signals**: Fine-grained reactivity with `futures-signals`
- **Async Support**: Guest-side async/await for animations and effects

---

## Quick Start

### Prerequisites

- Rust 1.75+ with `wasm32-unknown-unknown` target
- For Android: Android SDK + NDK
- For iOS: Xcode

### Build & Run

```bash
# macOS
./build_mac.sh

# Android
./build_android.sh
cd android && ./gradlew assembleDebug

# Web
./build_web.sh
cd web && python3 -m http.server 8000
# Open http://localhost:8000
```

---

## Project Structure

```
.
├── crates/
│   ├── dyxel-core/          # Host engine core
│   ├── dyxel-render-api/    # Render abstraction
│   ├── dyxel-render-vello/  # Vello renderer
│   ├── dyxel-render-impeller/ # Impeller renderer
│   ├── dyxel-shared/        # Shared types & protocol
│   ├── dyxel-view/          # Guest UI framework
│   └── wasm3/               # WASM3 runtime
├── sample/                  # Example guest app
├── mac/                     # macOS host
├── web/                     # Web host
├── android/                 # Android host
├── docs/
│   └── zh-cn/               # Chinese documentation
├── assets/                  # Logo and resources
└── build_*.sh               # Build scripts
```

---

## License

This project is dual-licensed under either:

- **Apache License, Version 2.0** ([LICENSE-APACHE](LICENSE-APACHE) or https://www.apache.org/licenses/LICENSE-2.0)
- **MIT License** ([LICENSE-MIT](LICENSE-MIT) or https://opensource.org/licenses/MIT)

at your option. See [LICENSE](LICENSE) for details.

---

## Acknowledgments

- [Vello](https://github.com/linebender/vello) - GPU-accelerated 2D graphics
- [Taffy](https://github.com/DioxusLabs/taffy) - Flexbox layout engine
- [wgpu](https://github.com/gfx-rs/wgpu) - WebGPU implementation
- [wasm3](https://github.com/wasm3/wasm3) - High-performance WASM interpreter

---

<p align="center">
  <i>Build dynamic, ship fast, run everywhere.</i>
</p>
