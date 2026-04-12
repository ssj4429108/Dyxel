#!/bin/bash
set -e

# Use dev-fast profile for quicker development builds
# Set DYXEL_RELEASE=1 to use full release builds for production
PROFILE="${DYXEL_PROFILE:-dev-fast}"

echo "Using profile: $PROFILE (set DYXEL_PROFILE=release for full optimization)"

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
if [ "$PROFILE" = "release" ]; then
    cargo build -p dyxel-mac --release
    BINARY="./target/release/dyxel-mac"
else
    cargo build -p dyxel-mac --profile dev-fast
    BINARY="./target/dev-fast/dyxel-mac"
fi

# 4. Run
echo "Running dyxel-mac with RUST_LOG=info..."
cp target/mac_dist/guest.wasm .
RUST_LOG=info DYXEL_DEBUG_FRAMES=1 $BINARY
