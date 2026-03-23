#!/bin/bash
set -e

# Configuration parameters
LIB_NAME="host_core"
OUTPUT_DIR="target/ios_dist"
SWIFT_OUT_DIR="target/ios_dist/Swift"
XCFRAMEWORK_NAME="HostCore.xcframework"

echo "--- 1. Building Guest WASM (sample) ---"
cd sample
RUSTFLAGS="-C target-feature=+bulk-memory,+mutable-globals,+nontrapping-fptoint -Clink-arg=--export=main" \
cargo build --release --target wasm32-unknown-unknown
cd ..

mkdir -p "$OUTPUT_DIR"
cp target/wasm32-unknown-unknown/release/sample.wasm "$OUTPUT_DIR/guest.wasm"
echo "Guest WASM copied to $OUTPUT_DIR."

echo "--- 2. Building Rust Libraries for iOS ---"
# Compile device and simulator architectures
# Note: This requires full Xcode installation and running in the environment
rustup target add aarch64-apple-ios aarch64-apple-ios-sim

echo "Building for aarch64-apple-ios (iPhone)..."
cargo build -p host-core --release --target aarch64-apple-ios

echo "Building for aarch64-apple-ios-sim (Simulator)..."
cargo build -p host-core --release --target aarch64-apple-ios-sim

echo "--- 3. Generating UniFFI Bindings ---"
# Get the compiled static library path
# We need to generate C header files required for Swift bindings
LIB_PATH="target/aarch64-apple-ios/release/lib${LIB_NAME}.a"

mkdir -p "$SWIFT_OUT_DIR"
# Generate Swift bindings
cargo run -p host-core --bin uniffi-bindgen generate --library "$LIB_PATH" --language swift --out-dir "$SWIFT_OUT_DIR" --no-format

echo "--- 4. Creating XCFramework ---"
# Clean up old XCFramework
rm -rf "$OUTPUT_DIR/$XCFRAMEWORK_NAME"

# Create XCFramework containing device and simulator
# Note: UniFFI also generates a modulemap and headers, which need to be properly packaged
xcodebuild -create-xcframework \
    -library "target/aarch64-apple-ios/release/lib${LIB_NAME}.a" \
    -headers "$SWIFT_OUT_DIR" \
    -library "target/aarch64-apple-ios-sim/release/lib${LIB_NAME}.a" \
    -headers "$SWIFT_OUT_DIR" \
    -output "$OUTPUT_DIR/$XCFRAMEWORK_NAME"

echo "--- Build Complete! ---"
echo "XCFramework: $OUTPUT_DIR/$XCFRAMEWORK_NAME"
echo "Swift Bindings: $SWIFT_OUT_DIR"
echo "WASM Asset: $OUTPUT_DIR/guest.wasm"
echo ""
echo "Now you can drag 'HostCore.xcframework' and the Swift files into your Xcode project."
