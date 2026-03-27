# Android 性能监控 - 实现总结

## 已完成的工作

### 1. 性能监控代码 (自动生效)

**位置**: `crates/dyxel-render-vello/src/lib.rs`

- ✅ 详细帧时间分解 (init_done → perf_start → state_lock → scene_build → gpu_render → blit_submit → present_return)
- ✅ 每 60 帧输出 FPS/GPU 时间/Present 时间
- ✅ 每 300 帧输出完整时间线
- ✅ Android 特定日志标签 `[DIAG-Android]`，包含内存和 CPU 使用率

**示例输出**:
```
[DIAG-Android] Frame 60: Total=16.67ms, GPU=3.2ms, Present=13.4ms, FPS=60.0, Mem=45.2MB, CPU=12.3%
```

### 2. 系统信息获取

**位置**: `crates/dyxel-perf/src/platform/android.rs`

- ✅ 内存使用: 读取 `/proc/self/status` (VmRSS)
- ✅ CPU 使用率: 读取 `/proc/self/stat` (utime + stime)
- ✅ 备用方案: `/proc/self/statm` (如果 status 不可用)

### 3. Overlay 显示

- ✅ 按 P 键显示性能覆盖层
- ✅ 缓存 Editor 避免重复创建
- ✅ 显示 FPS / 帧时间 / 内存 / CPU

### 4. 监控脚本

**文件**: `scripts/android_perf.sh`

```bash
# 实时监控
./scripts/android_perf.sh monitor

# 查看最近 FPS
./scripts/android_perf.sh fps

# 完整性能报告
./scripts/android_perf.sh full

# 抓取 systrace
./scripts/android_perf.sh systrace
```

### 5. 文档

**文件**: `docs/android_perf.md`

- 快速开始指南
- 性能指标解读
- 常见问题 FAQ
- 调试命令参考

## Android 构建配置

**Cargo.toml** (`crates/dyxel-core/Cargo.toml`):
```toml
[target.'cfg(target_os = "android")'.dependencies]
android_logger = "0.13"
ndk-sys = "0.6"
jni = "0.21"
```

**日志初始化** (`crates/dyxel-core/src/platform.rs`):
```rust
android_logger::init_once(
    android_logger::Config::default().with_max_level(log::LevelFilter::Info),
);
```

## 使用步骤

### 1. 构建 Android App

```bash
./build_android.sh
```

### 2. 安装并运行

```bash
adb install -r android/app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n com.dyxel.app/.MainActivity
```

### 3. 查看性能日志

```bash
# 实时监控
./scripts/android_perf.sh monitor

# 或手动查看
adb logcat -s dyxel:D *:S
```

### 4. 开启 Overlay

在 App 中按 **P 键** (或通过 adb 发送按键事件):
```bash
adb shell input keyevent 44  # KEYCODE_P
```

## 性能数据说明

| 指标 | 正常范围 | 说明 |
|-----|---------|-----|
| FPS | 60 | Android 通常锁定 60 FPS (VSync) |
| GPU | < 5ms | GPU 渲染时间 |
| Present | ~13ms | VSync 等待时间 |
| Mem | 20-100MB | 进程内存使用 |
| CPU | 10-30% | 进程 CPU 使用率 |

## 注意事项

1. **VSync 限制**: Android 系统强制 VSync，即使代码设置 `PresentMode::Immediate`
   - 大多数设备: 60 FPS
   - 高刷设备: 90/120 FPS (需在系统设置中开启)

2. **日志过滤**: Android 日志较多，建议使用过滤器:
   ```bash
   adb logcat -s dyxel:D *:S
   ```

3. **性能衰退**: 长时间运行后如果 FPS 下降，可能是:
   - 热节流 (CPU/GPU 降频)
   - 内存压力 (检查 Mem 值)
   - 资源泄漏 (检查内存是否持续增长)

## 后续优化建议

1. **降低 Logic 线程频率**: Android 上可适当降低 tick 频率以节省电量
2. **纹理压缩**: 使用 ETC2/ASTC 压缩减少 GPU 带宽
3. **内存池**: 复用纹理/缓冲区减少分配开销
4. **帧率自适应**: 根据设备温度动态调整目标帧率

## 相关文件

- `crates/dyxel-perf/src/platform/android.rs` - Android 系统信息获取
- `crates/dyxel-render-vello/src/lib.rs` - 渲染性能监控
- `crates/dyxel-core/src/platform.rs` - Android 日志初始化
- `scripts/android_perf.sh` - 监控脚本
- `docs/android_perf.md` - 详细文档
