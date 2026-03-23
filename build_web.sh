#!/bin/bash
set -e

# 1. Enter project root directory
cd "$(dirname "$0")"

# 2. Build rendering engine (dyxel-web)
# Use wasm-pack to build web target, output to web/pkg
wasm-pack build ./web --target web --out-dir ./pkg --out-name dyxel_web

# 3. Build application business logic (sample)
# This is a standalone WASM module
cargo build --target wasm32-unknown-unknown -p sample --release
cp target/wasm32-unknown-unknown/release/sample.wasm web/sample.wasm

echo "Build complete! Please serve the 'web/' directory using a local server (e.g., 'python3 -m http.server')."
