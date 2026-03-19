#!/bin/bash
set -e

# 1. 编译 WASM 模块 (sample)
echo "Building sample.wasm..."
cd sample
# 移除显式导出，让所有 #[no_mangle] 符号自动导出
RUSTFLAGS="-C target-feature=+bulk-memory,+mutable-globals,+nontrapping-fptoint" \
cargo build --release --target wasm32-unknown-unknown

# 2. 拷贝并处理
cd ..
mkdir -p target/mac_dist
cp target/wasm32-unknown-unknown/release/sample.wasm target/mac_dist/guest.wasm

# 3. 编译 Native Host (macOS)
echo "Building host-mac..."
cargo build -p host-mac --release

# 4. 运行
echo "Running host-mac..."
cp target/mac_dist/guest.wasm .
./target/release/host-mac
