#!/bin/bash
set -e

# 配置参数
TARGET_ARCH="arm64-v8a" # 可选: arm64-v8a, armeabi-v7a, x86_64, x86
API_LEVEL=24
ANDROID_PROJECT_DIR="android"
JNI_LIBS_DIR="$ANDROID_PROJECT_DIR/app/src/main/jniLibs"
KOTLIN_OUT_DIR="$ANDROID_PROJECT_DIR/app/src/main/java"
ASSETS_DIR="$ANDROID_PROJECT_DIR/app/src/main/assets"

echo "--- 1. Building Guest WASM (sample) ---"
cd sample
RUSTFLAGS="-C target-feature=+bulk-memory,+mutable-globals,+nontrapping-fptoint" \
cargo build --release --target wasm32-unknown-unknown
cd ..

mkdir -p "$ASSETS_DIR"
cp target/wasm32-unknown-unknown/release/sample.wasm "$ASSETS_DIR/guest.wasm"
echo "Guest WASM copied to assets."

echo "--- 2. Building Native Host (Android $TARGET_ARCH) ---"
# 使用 cargo-ndk 进行交叉编译，并增加 16KB 页大小对齐支持
RUSTFLAGS="-C link-arg=-z -C link-arg=max-page-size=16384" \
cargo ndk -t "$TARGET_ARCH" -P "$API_LEVEL" -o "$JNI_LIBS_DIR" build -p host-android --release

echo "--- 3. Generating UniFFI Kotlin Bindings ---"
# 获取编译出的 .so 路径 (针对 arm64-v8a)
SO_PATH="target/aarch64-linux-android/release/libhost_android.so"

# 使用 host-core 中定义的 uniffi-bindgen 工具生成绑定
cargo run -p host-core --bin uniffi-bindgen generate --library "$SO_PATH" --language kotlin --out-dir crates/host-core/generated --no-format

# 创建包结构目录并拷贝文件
mkdir -p "$KOTLIN_OUT_DIR/uniffi/host_core"
cp crates/host-core/generated/uniffi/host_core/host_core.kt "$KOTLIN_OUT_DIR/uniffi/host_core/host_core.kt"

echo "--- Build Complete! ---"
echo "Native Library: $JNI_LIBS_DIR/$TARGET_ARCH/libhost_android.so"
echo "Kotlin Bindings: $KOTLIN_OUT_DIR/uniffi/host_core/host_core.kt"
echo "WASM Asset: $ASSETS_DIR/guest.wasm"
echo ""
echo "Now you can run './gradlew assembleDebug' in the '$ANDROID_PROJECT_DIR' directory."
