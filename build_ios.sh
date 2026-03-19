#!/bin/bash
set -e

# 配置参数
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
# 编译真机和模拟器架构
# 注意：这需要安装完整的 Xcode 并在环境下运行
rustup target add aarch64-apple-ios aarch64-apple-ios-sim

echo "Building for aarch64-apple-ios (iPhone)..."
cargo build -p host-core --release --target aarch64-apple-ios

echo "Building for aarch64-apple-ios-sim (Simulator)..."
cargo build -p host-core --release --target aarch64-apple-ios-sim

echo "--- 3. Generating UniFFI Bindings ---"
# 获取编译出的静态库路径
# 我们需要生成 Swift 绑定所需的 C 头文件
LIB_PATH="target/aarch64-apple-ios/release/lib${LIB_NAME}.a"

mkdir -p "$SWIFT_OUT_DIR"
# 生成 Swift 绑定
cargo run -p host-core --bin uniffi-bindgen generate --library "$LIB_PATH" --language swift --out-dir "$SWIFT_OUT_DIR" --no-format

echo "--- 4. Creating XCFramework ---"
# 清理旧的 XCFramework
rm -rf "$OUTPUT_DIR/$XCFRAMEWORK_NAME"

# 创建包含真机和模拟器的 XCFramework
# 注意：UniFFI 还会生成一个 modulemap 和头文件，需要正确打包
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
