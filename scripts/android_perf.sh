#!/bin/bash
# Android 性能监控脚本

PACKAGE_NAME="com.dyxel.app"  # 请替换为实际的包名

echo "========================================"
echo "Dyxel Android 性能监控工具"
echo "========================================"
echo ""

# 检查设备连接
if ! adb devices | grep -q "device$"; then
    echo "错误: 没有检测到 Android 设备"
    echo "请连接设备或启动模拟器"
    exit 1
fi

echo "设备已连接:"
adb shell getprop ro.product.model
echo ""

# 功能选择
case "${1:-monitor}" in
    monitor)
        echo "启动实时性能监控..."
        echo "按 Ctrl+C 停止"
        echo ""
        adb logcat -c  # 清空日志
        adb logcat -s dyxel:D RustStdoutStderr:D *:S | grep -E "(DIAG|Frame|FPS|GPU)"
        ;;
    
    fps)
        echo "提取 FPS 统计..."
        adb logcat -d -s dyxel:D | grep "DIAG" | tail -20
        ;;
    
    mem)
        echo "当前内存使用:"
        adb shell dumpsys meminfo "$PACKAGE_NAME" | grep -E "(TOTAL|Native Heap|Dalvik Heap)"
        ;;
    
    gfx)
        echo "GPU 渲染统计:"
        adb shell dumpsys gfxinfo "$PACKAGE_NAME" | head -50
        ;;
    
    cpu)
        echo "CPU 使用统计:"
        adb shell top -n 1 -p $(adb shell pidof "$PACKAGE_NAME") 2>/dev/null || \
        adb shell ps | grep "$PACKAGE_NAME"
        ;;
    
    full)
        echo "完整性能报告:"
        echo ""
        echo "=== 帧率统计 ==="
        adb logcat -d -s dyxel:D | grep "DIAG" | tail -5
        echo ""
        
        echo "=== 内存使用 ==="
        adb shell dumpsys meminfo "$PACKAGE_NAME" 2>/dev/null | head -30
        echo ""
        
        echo "=== GPU 信息 ==="
        adb shell dumpsys gpu | head -20
        echo ""
        ;;
    
    systrace)
        echo "抓取 10 秒 systrace..."
        adb shell atrace gfx input view wm am res --time=10 -o /data/local/tmp/dyxel_trace.txt &
        echo "抓取中... 请操作 App"
        sleep 10
        adb pull /data/local/tmp/dyxel_trace.txt /tmp/
        echo "完成: /tmp/dyxel_trace.txt"
        echo "请在 Chrome 中打开: chrome://tracing"
        ;;
    
    *)
        echo "用法: $0 [command]"
        echo ""
        echo "命令:"
        echo "  monitor  - 实时性能监控 (默认)"
        echo "  fps      - 查看 FPS 统计"
        echo "  mem      - 查看内存使用"
        echo "  gfx      - 查看 GPU 渲染信息"
        echo "  cpu      - 查看 CPU 使用"
        echo "  full     - 完整性能报告"
        echo "  systrace - 抓取 systrace 分析"
        echo ""
        echo "示例:"
        echo "  $0 monitor    # 实时监控"
        echo "  $0 fps        # 查看最近 FPS"
        echo "  $0 full       # 完整报告"
        ;;
esac
