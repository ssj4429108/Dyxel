# Android 性能监控指南

## 快速开始

Android 构建已默认启用性能监控，无需额外配置。

### 查看实时性能数据

```bash
# 连接设备后查看日志
adb logcat -s dyxel:D *:S

# 或查看完整日志（包含性能诊断）
adb logcat -s dyxel:D RustStdoutStderr:D
```

### 日志标签说明

| 标签 | 含义 | 示例 |
|-----|------|-----|
| `[DIAG]` | 帧时间诊断 | `Frame 60: Total=16.67ms, GPU=3.2ms, Present=13.4ms` |
| `RenderThread` | 渲染线程状态 | `Creating surface id: 1, size: 1080x1920` |
| `LogicThread` | 逻辑线程状态 | `WASM tick failed: ...` |
| `VelloBackend` | GPU 渲染后端 | `VSync disabled by default` |

## 性能指标解读

### 正常范围

```
[DIAG] Frame 60: Total=16.67ms, GPU=3.2ms, Present=13.4ms, Reported FPS=60.0

分析:
- Total=16.67ms → 帧时间正常 (60 FPS = 16.67ms)
- GPU=3.2ms → GPU 渲染很快
- Present=13.4ms → VSync 等待时间 (Android 通常强制 VSync)
- FPS=60.0 → 受显示器刷新率限制
```

### Android 特殊考虑

1. **VSync 限制**: Android 系统通常强制 VSync，即使设置 `PresentMode::Immediate`
   - 大多数设备锁定 60 FPS
   - 高刷设备 (90/120Hz) 可能达到更高帧率

2. **热节流**: 长时间高帧率可能导致:
   - CPU/GPU 降频
   - 帧率从 60 降到 30

3. **内存压力**: Android 系统会主动回收内存
   - 监控 `Mem:` 值的增长趋势
   - 如果持续增长，可能有内存泄漏

## 高级监控

### 1. 详细阶段分析

每 300 帧 (约 5 秒) 输出完整分解:

```
=== Frame Timing Breakdown ===
  init_done            -> perf_start          : 0.001 ms
  perf_start           -> state_lock          : 0.217 ms
  state_lock           -> scene_build         : 0.820 ms
  scene_build          -> gpu_render          : 1.248 ms  ← GPU 渲染
  gpu_render           -> blit_submit         : 0.168 ms
  blit_submit          -> present_return      : 0.025 ms  ← VSync 等待
  --------------------------------
  TOTAL FRAME TIME: 2.482 ms (402.9 FPS)
```

### 2. 系统信息监控

Overlay 显示 (按 P 键开启):
```
FPS: 59.8
Frame: 16.72ms
Mem: 45.2MB      ← Android 进程内存
CPU: 12.3%       ← 进程 CPU 使用率
```

### 3. 使用 Android Studio Profiler

```bash
# 1. 构建 release 版本
./build_android.sh

# 2. 安装并运行
adb install -r android/app/build/outputs/apk/debug/app-debug.apk

# 3. 在 Android Studio 中:
#    - 打开 Profiler
#    - 选择 CPU 和 Memory 监控
#    - 查看 GPU 渲染时间线
```

## 性能优化建议

### GPU 优化

```rust
// 在 Android 上，优先使用:
- 减少纹理尺寸 (使用 mipmapping)
- 避免每帧创建/销毁资源
- 复用 Scene 对象
```

### CPU 优化

```rust
// Logic 线程 tick 频率
const LOGIC_TICK_RATE: u64 = 16; // 60 FPS

// Android 上可以适当降低以节省电量
const LOGIC_TICK_RATE: u64 = 33; // 30 FPS (更省电)
```

### 内存优化

监控 `dyxel-perf` 报告的内存使用:
```
Mem: 45.2MB

如果持续增长:
- 检查是否有未释放的 Surface
- 检查 Editor/FontSystem 缓存是否无限增长
```

## 常见问题

### Q: 为什么帧率锁定在 60 FPS？
A: Android 系统强制 VSync，即使代码设置 `PresentMode::Immediate`。
高刷设备 (120Hz) 需要在系统设置中开启高刷新率。

### Q: 为什么内存使用持续增长？
A: 检查:
1. 热重启后是否正确清理 SharedState
2. Editor 缓存是否设置上限
3. 纹理资源是否正确释放

### Q: 如何诊断卡顿？
A: 查看详细日志:
```bash
adb logcat -s dyxel:D | grep -E "(Frame|GPU|Present)"
```

如果 `scene_build -> gpu_render` 时间突然增加，可能是:
- 布局计算耗时增加 (节点过多)
- 文本重新布局 (内容频繁变化)

## 调试命令

```bash
# 实时监控 FPS
adb shell dumpsys gfxinfo <package_name>

# 查看 GPU 使用情况
adb shell dumpsys gpu

# 查看内存使用
adb shell dumpsys meminfo <package_name>

# 抓取 systrace (性能分析)
adb shell atrace gfx input view wm am sm --time=10 -o /data/local/tmp/trace.txt
adb pull /data/local/tmp/trace.txt .
# 在 Chrome 中打开: chrome://tracing
```
