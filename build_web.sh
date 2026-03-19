#!/bin/bash
set -e

# 1. 进入项目根目录
cd "$(dirname "$0")"

# 2. 构建渲染引擎 (host-web)
# 使用 wasm-pack 构建 web 目标，输出到 web/pkg
wasm-pack build crates/host-web --target web --out-dir ../../web/pkg

# 3. 构建应用业务 (sample)
# 这是一个独立的 WASM 模块
cargo build --target wasm32-unknown-unknown -p sample --release
cp target/wasm32-unknown-unknown/release/sample.wasm web/sample.wasm

echo "Build complete! Please serve the 'web/' directory using a local server (e.g., 'python3 -m http.server')."
