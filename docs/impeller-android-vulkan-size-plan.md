# Impeller Android Vulkan 渲染尺寸问题方案记录

日期：2026-05-06

## 背景

当前 `dyxel-render-impeller` 在 Android GLES WSI 模式下已经可以正常渲染，但切到 Vulkan WSI 后出现明显的坐标/尺寸异常：

- 一个原本应为小尺寸的矩形会被渲染成接近全屏；
- 元素位置也可能被放大或偏移；
- 现象更像是 Vulkan render target / swapchain 的逻辑尺寸不对，而不是普通 layout 计算错误。

## 当前本地路径

相关代码：

- `crates/dyxel-render-impeller/src/runtime.rs`
- `crates/dyxel-render-impeller/src/backend.rs`
- `crates/dyxel-core/src/bridge.rs`
- `android/app/src/main/java/com/dyxel/android/MainActivity.kt`

当前 GLES 路径会显式传入 framebuffer 尺寸：

```rust
context.wrap_fbo(
    0,
    PixelFormat::RGBA8888,
    ISize::new(record.width as i64, record.height as i64),
)
```

而当前 Vulkan 路径是：

```rust
context.create_new_vulkan_swapchain(vk_surface)
swapchain.acquire_next_surface_new()
```

这里只把 `VkSurfaceKHR` 交给 Impeller，没有显式传入 `width / height`。如果 Impeller/Android WSI 推断出的 swapchain extent 异常，例如接近 `1x1` 或逻辑坐标被当作 `0..1`，就会导致小矩形覆盖整个屏幕。

## Flutter Android Impeller 参考

Flutter Android 上的 Impeller 并不是简单地由 app 侧创建 `VkSurfaceKHR` 后直接交给渲染器。它更接近：

```text
Android Platform
  -> AndroidContextVKImpeller
      -> Impeller ContextVK
  -> AndroidSurfaceVKImpeller
      -> SurfaceContextVK
          -> SwapchainVK::Create(parent_context, ANativeWindow)
              -> Android AHB swapchain if available
              -> otherwise KHRSwapchainVK
  -> GPUSurfaceVulkanImpeller
      -> acquire drawable
      -> draw
      -> present
```

也就是说 Flutter 的 Android Vulkan path 是 `ANativeWindow` aware 的，内部负责：

- native window 尺寸；
- swapchain extent；
- resize；
- surface teardown；
- Android hardware buffer / KHR fallback。

而 `impeller-rs` 当前暴露的 standalone C API 更窄：

```text
VkSurfaceKHR -> ImpellerVulkanSwapchainCreateNew
```

它没有直接暴露 Flutter Android 内部的 `SurfaceContextVK + SwapchainVK::Create(parent_context, ANativeWindow)` 路径。

参考：

- Flutter Impeller 文档：<https://docs.flutter.dev/perf/impeller>
- Flutter Android Vulkan surface：<https://flutter.googlesource.com/mirrors/engine/+/refs/heads/main/shell/platform/android/android_surface_vk_impeller.cc>
- Flutter Vulkan swapchain：<https://api.flutter.dev/impeller/renderer_2backend_2vulkan_2swapchain_2swapchain__vk_8cc_source.html>
- impeller-rs Vulkan swapchain API：<https://docs.rs/impellers/latest/impellers/struct.VkSwapChain.html>

## 当前判断

优先怀疑：

1. Vulkan swapchain/render-target 的逻辑尺寸错误；
2. `VkSurfaceCapabilitiesKHR.currentExtent` 或 Impeller 内部 surface size 与 Java `SurfaceView` 尺寸不一致；
3. C API standalone Vulkan path 缺少 Android native window/explicit size 信息；
4. surface/swapchain 创建线程与使用线程和 Flutter Android 的模型不同，可能放大生命周期/尺寸同步问题。

不优先怀疑：

- Taffy layout 本身；
- RenderPackage 节点尺寸本身；
- 普通 DisplayList rect/path API 参数错误。

## 验证方案

### 1. 使用已有 normalized probe

已有探针：

```text
debug.dyxel.impeller_probe=mini-normalized
debug.dyxel.impeller_probe=scene-normalized
```

它会在 DisplayList root 上做：

```rust
builder.scale(
    1.0 / viewport_width,
    1.0 / viewport_height,
);
```

如果开启后小矩形尺寸/位置恢复正常，基本可以确认：

```text
Vulkan Impeller 当前把画布当成 1x1 或类似归一化坐标系处理。
```

### 2. 增强 Vulkan surface/swapchain 日志

建议补充/确认日志：

- Java `surfaceChanged(width, height)`；
- `ANativeWindow_getWidth/Height/Format`；
- `VkSurfaceCapabilitiesKHR.currentExtent`；
- `minImageExtent / maxImageExtent`；
- surface format；
- present mode；
- graphics queue family 是否支持 present；
- swapchain recreate 前后的尺寸。

重点观察：

```text
currentExtent = 1x1
currentExtent = 0xFFFFFFFF x 0xFFFFFFFF
ANativeWindow size != Java SurfaceView size
ANativeWindow size != package.viewport
```

## 方案分层

### 方案 A：短期 root-scale workaround

如果 normalized probe 验证成立，可以先在 Android Vulkan WSI 下默认增加一层 root scale：

```rust
builder.scale(
    1.0 / package.viewport.0.max(1) as f32,
    1.0 / package.viewport.1.max(1) as f32,
);
```

优点：

- 改动最小；
- 可以快速让画面尺寸/位置恢复可用；
- 便于继续验证其他 Impeller 功能。

缺点：

- 这是 workaround，不是根因修复；
- 如果后续 swapchain extent 修正，这个 scale 需要关闭，否则画面会变得过小；
- 必须 gated，例如只在 Android Vulkan 且显式开关启用时生效。

建议开关：

```text
debug.dyxel.impeller_vulkan_root_scale=1
```

### 方案 B：修正现有 impeller-rs KHR Vulkan 接入

保持当前 `VkSurfaceKHR -> ImpellerVulkanSwapchainCreateNew` 路径，但尽量贴近 Flutter 的线程和生命周期模型：

1. Android Impeller Vulkan 下，把 swapchain 创建迁移到 render thread；
2. `surfaceDestroyed / stop_native` 时真正调用 runtime suspend/clear，确保 Vulkan swapchain 先于 `ANativeWindow_release` 释放；
3. Vulkan 路径先不要主动 `ANativeWindow_setBuffersGeometry`，或将其变成实验开关；
4. resize 时不要盲目重建，先确认 Impeller swapchain 对 resize 的自管理能力与 currentExtent 日志。

优点：

- 不需要 fork Impeller；
- 仍使用 impeller-rs 官方 C API；
- 可以修正潜在线程/lifecycle 问题。

缺点：

- 如果 C API 本身无法传 explicit size，仍可能无法根治尺寸问题。

### 方案 C：patch impeller-rs / Impeller C API，增加 Android Vulkan explicit-size 入口

增加类似：

```c
ImpellerAndroidVulkanSwapchainCreateNew(
    ImpellerContext context,
    ANativeWindow* window,
    int width,
    int height
)
```

或至少给现有 Vulkan swapchain create 加 explicit size/extent。

优点：

- 可以直接解决 standalone C API 缺少尺寸信息的问题；
- 比完整复刻 Flutter Android SurfaceContextVK 成本低。

缺点：

- 需要维护自定义 `libimpeller.so` 或等待 upstream；
- 需要重新生成 Rust bindings。

### 方案 D：更贴近 Flutter，暴露 SurfaceContextVK Android 路径

自建 Impeller C wrapper，直接走 Flutter 内部：

```cpp
SurfaceContextVK
SwapchainVK::Create(parent_context, ANativeWindow)
```

优点：

- 最接近 Flutter Android production path；
- 能复用 Android AHB/KHR fallback、resize、teardown 等逻辑。

缺点：

- 成本最高；
- 需要维护自定义 Impeller build；
- API/ABI 维护成本较大。

## 当前倾向

短期建议：

1. 先使用 `mini-normalized / scene-normalized` 验证尺寸假设；
2. 若验证成立，先实现 gated root-scale workaround；
3. 同时补 Vulkan surface/swapchain extent 日志；
4. 后续再决定是否进入方案 B/C/D。

中期优先级建议：

```text
A: root-scale workaround + diagnostics
  -> B: 修正线程/lifecycle/swapchain teardown
    -> C: patch impeller-rs explicit Android Vulkan size API
      -> D: 自定义 Flutter SurfaceContextVK wrapper
```

