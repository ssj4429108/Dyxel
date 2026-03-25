#!/bin/bash
set -e

# Configuration parameters
TARGET_ARCH="arm64-v8a" # Options: arm64-v8a, armeabi-v7a, x86_64, x86
API_LEVEL=24

# NDK Configuration: Set ANDROID_NDK_VERSION to specify NDK version (e.g., "27.1.12297006")
# Or directly set ANDROID_NDK_HOME environment variable
ANDROID_NDK_VERSION="${ANDROID_NDK_VERSION:-27.1.12297006}"

if [ -z "$ANDROID_NDK_HOME" ]; then
    # Try to find NDK from ANDROID_SDK_HOME or ANDROID_SDK_ROOT
    SDK_ROOT="${ANDROID_SDK_HOME:-${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}"
    if [ -d "$SDK_ROOT/ndk/$ANDROID_NDK_VERSION" ]; then
        export ANDROID_NDK_HOME="$SDK_ROOT/ndk/$ANDROID_NDK_VERSION"
    else
        echo "Error: ANDROID_NDK_HOME not set and NDK $ANDROID_NDK_VERSION not found in $SDK_ROOT/ndk/"
        echo "Please either:"
        echo "  1. Set ANDROID_NDK_HOME environment variable"
        echo "  2. Set ANDROID_NDK_VERSION to the installed NDK version"
        echo "  3. Install NDK $ANDROID_NDK_VERSION"
        exit 1
    fi
fi

echo "Using Android NDK: $ANDROID_NDK_HOME"
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
