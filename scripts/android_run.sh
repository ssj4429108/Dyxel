#!/bin/bash
# Android 构建、安装、运行和监控一体化脚本

set -e

PACKAGE_NAME="com.dyxel.app"
APK_PATH="android/app/build/outputs/apk/debug/app-debug.apk"

echo "========================================"
echo "Dyxel Android 性能测试工具"
echo "========================================"
echo ""

# 检查设备
if ! adb devices | grep -q "device$"; then
    echo "❌ 错误: 没有检测到 Android 设备"
    exit 1
fi

echo "✓ 设备已连接: $(adb shell getprop ro.product.model)"
echo ""

# 构建
if [ "${1:-}" = "--skip-build" ]; then
    echo "⏭  跳过构建"
else
    echo "🔨 1. 构建 Rust 库..."
    cd /Users/skipper/axzo/ai/test/taffy_vello_sync
    ./build_android.sh
    
    echo "📦 2. 构建 Android APK..."
    cd android
    ./gradlew assembleDebug
    cd ..
fi

# 安装
echo "📲 3. 安装 APK..."
adb install -r "$APK_PATH"

# 启动
echo "🚀 4. 启动应用..."
adb shell am start -n "$PACKAGE_NAME/.MainActivity"

# 等待应用启动
sleep 2

echo ""
echo "========================================"
echo "性能监控开始"
echo "========================================"
echo ""
echo "日志标签说明:"
echo "  [DIAG-Android] - 帧率/FPS/内存/CPU"
echo "  [Android-Perf] - 初始化日志"
echo "  RenderThread   - 渲染状态"
echo "  LogicThread    - 逻辑状态"
echo ""
echo "按 Ctrl+C 停止监控"
echo ""

# 清空旧日志
adb logcat -c

# 监控日志
adb logcat -s dyxel:D RustStdoutStderr:D *:S | grep -E "(DIAG|Android-Perf|Frame|FPS|GPU|RenderThread|LogicThread)"
