#!/bin/bash
set -e

# macOS 环境自动设置 LIBCLANG_PATH 解决 bindgen 找不到 libclang.dylib 的问题
if [ "$(uname)" == "Darwin" ]; then
    if command -v brew &> /dev/null; then
        export LIBCLANG_PATH="$(brew --prefix llvm)/lib"
    fi
fi

echo "Checking Android (aarch64)..."
cargo ndk -t arm64-v8a check

echo "Checking macOS (Apple Silicon)..."
cargo check --target aarch64-apple-darwin

echo "Checking Web (WASM)..."
cargo check --target wasm32-unknown-unknown

echo "All checks passed!"
