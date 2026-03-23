#!/bin/bash
set -e

# Configuration parameters
TARGET_ARCH="arm64-v8a" # Options: arm64-v8a, armeabi-v7a, x86_64, x86
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
# Use cargo-ndk for cross-compilation, with 16KB page size alignment support
# Build target changed to dyxel-core, explicitly enable vello and wasm3-support features
RUSTFLAGS="-C link-arg=-z -C link-arg=max-page-size=16384" \
cargo ndk -t "$TARGET_ARCH" -P "$API_LEVEL" -o "$JNI_LIBS_DIR" build -p dyxel-core --release --features vello,wasm3-support

echo "--- 3. Generating UniFFI Kotlin Bindings ---"
# Get the compiled .so path (generated from dyxel-core)
SO_PATH="target/aarch64-linux-android/release/libdyxel_core.so"

# Use uniffi-bindgen tool defined in dyxel-core to generate bindings
cargo run -p dyxel-core --bin uniffi-bindgen generate --library "$SO_PATH" --language kotlin --out-dir crates/dyxel-core/generated --no-format

# Create package structure directory and copy files
mkdir -p "$KOTLIN_OUT_DIR/uniffi/dyxel_core"
cp crates/dyxel-core/generated/uniffi/dyxel_core/dyxel_core.kt "$KOTLIN_OUT_DIR/uniffi/dyxel_core/dyxel_core.kt"

echo "--- Build Complete! ---"
echo "Native Library: $JNI_LIBS_DIR/$TARGET_ARCH/libdyxel_core.so"
echo "Kotlin Bindings: $KOTLIN_OUT_DIR/uniffi/dyxel_core/dyxel_core.kt"
echo "WASM Asset: $ASSETS_DIR/guest.wasm"
echo ""
echo "Now you can run './gradlew assembleDebug' in the '$ANDROID_PROJECT_DIR' directory."
