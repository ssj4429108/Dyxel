#!/bin/bash
set -e

# 1. Compile WASM module (sample)
echo "Building sample.wasm..."
cd sample
# Remove explicit exports, let all #[no_mangle] symbols be exported automatically
RUSTFLAGS="-C target-feature=+bulk-memory,+mutable-globals,+nontrapping-fptoint" \
cargo build --release --target wasm32-unknown-unknown

# 2. Copy and process
cd ..
mkdir -p target/mac_dist
cp target/wasm32-unknown-unknown/release/sample.wasm target/mac_dist/guest.wasm

# 3. Compile Native Host (macOS)
echo "Building dyxel-mac..."
cargo build -p dyxel-mac --release

# 4. Run
echo "Running dyxel-mac with RUST_LOG=info..."
cp target/mac_dist/guest.wasm .
RUST_LOG=info ./target/release/dyxel-mac
