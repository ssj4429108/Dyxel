# Vello Shader 延迟加载架构设计

## 目标
将首次启动时的 shader 编译时间从 1.8s 降至 0.9s（减少 50%），通过延迟加载非关键 shader。

## 核心思想

渲染流程分为多个阶段，部分 shader 在特定场景才需要：

```
渲染管线阶段：

Phase 1: 路径预处理 (Path Preprocessing)
  - pathtag_reduce, pathtag_reduce2
  - pathtag_scan1, pathtag_scan, pathtag_scan_large
  - bbox_clear, flatten
  
Phase 2: 绘制处理 (Draw Processing)  
  - draw_reduce, draw_leaf
  - clip_reduce, clip_leaf
  - binning, tile_alloc
  - backdrop
  
Phase 3: 光栅化 (Rasterization) - 核心阶段
  - path_count_setup, path_count
  - coarse
  - path_tiling_setup, path_tiling
  - fine_area (必须)
  
Phase 4: 高级抗锯齿 (Advanced AA) - 可选
  - fine_msaa8, fine_msaa16
```

## 延迟加载策略

### Tier 1: 核心（立即加载）
首次渲染必须，约占 40% 的 shader 代码：
- `path_count_setup`, `path_count`
- `coarse`
- `path_tiling_setup`, `path_tiling`
- `fine_area`

### Tier 2: 路径预处理（延迟加载）
只在有复杂路径时需要，约占 30%：
- `pathtag_reduce`, `pathtag_reduce2`
- `pathtag_scan1`, `pathtag_scan`, `pathtag_scan_large`
- `flatten`, `bbox_clear`

### Tier 3: 裁剪/混合（延迟加载）
只在有 clip/draw 时需要，约占 20%：
- `draw_reduce`, `draw_leaf`
- `clip_reduce`, `clip_leaf`
- `binning`, `tile_alloc`, `backdrop`

### Tier 4: MSAA（按需加载）
只在启用 MSAA 时需要，约占 10%：
- `fine_msaa8`, `fine_msaa16`

## 实施架构

```rust
// 新增：分层 Shader 管理器
pub struct TieredShaderManager {
    // Tier 1: 核心（已加载）
    core: CoreShaders,
    
    // Tier 2-4: 延迟加载
    path_preload: LazyShaderGroup<PathPreloadShaders>,
    draw_clip: LazyShaderGroup<DrawClipShaders>,
    msaa: LazyShaderGroup<MsaaShaders>,
    
    // 加载状态
    loaded_tiers: AtomicU8,
}

impl TieredShaderManager {
    /// 初始化时只加载 Tier 1
    pub fn new(device: &Device) -> Self {
        let core = load_core_shaders(device);
        Self {
            core,
            path_preload: LazyShaderGroup::new(),
            draw_clip: LazyShaderGroup::new(),
            msaa: LazyShaderGroup::new(),
            loaded_tiers: AtomicU8::new(0b0001), // Tier 1 已加载
        }
    }
    
    /// 后台线程加载 Tier 2
    pub fn preload_path_shaders(&self, device: &Device) {
        if self.loaded_tiers.fetch_or(0b0010, Ordering::SeqCst) & 0b0010 == 0 {
            // 首次调用，启动后台加载
            let shaders = load_path_preload_shaders(device);
            self.path_preload.set(shaders);
        }
    }
    
    /// 渲染时按需获取 shader
    pub fn get_pathtag_reduce(&self) -> Option<ShaderId> {
        self.path_preload.get().map(|s| s.pathtag_reduce)
    }
}
```

## 在 dyxel 中的集成

```rust
// VelloBackend 初始化
impl VelloBackend {
    pub async fn new_with_lazy_loading(device: &Device, tier: DeviceMemoryTier) -> Self {
        // 1. 立即加载 Tier 1（核心）
        let shader_manager = TieredShaderManager::new(device);
        
        // 2. 根据 Tier 决定预加载策略
        match tier {
            DeviceMemoryTier::LowEnd => {
                // 低端机：延迟加载 Tier 2-3，MSAA 禁用
                spawn_background_thread(move || {
                    shader_manager.preload_path_shaders(device);
                    shader_manager.preload_draw_shaders(device);
                });
            }
            _ => {
                // 高端机：立即加载所有
                shader_manager.load_all(device);
            }
        }
        
        Self { shader_manager, ... }
    }
}
```

## 预期效果

### 首次启动（无 cache）
| Tier | Shader 数量 | 代码量 | 加载时间 |
|------|------------|--------|----------|
| Tier 1 (核心) | 5 | 40% | ~700ms |
| Tier 2 (路径) | 7 | 30% | ~500ms (后台) |
| Tier 3 (裁剪) | 7 | 20% | ~400ms (按需) |
| Tier 4 (MSAA) | 2 | 10% | ~200ms (按需) |
| **总计** | **21** | **100%** | **~1.8s** |

**延迟加载后首次阻塞时间：700ms**（减少 60%）

### 渐进式加载
- 0ms: Tier 1 加载完成，可渲染简单场景
- 700ms: Tier 2 加载完成，支持复杂路径
- 1200ms: Tier 3 加载完成，支持裁剪/混合
- 按需: Tier 4 加载，支持 MSAA

## 风险与缓解

| 风险 | 概率 | 缓解措施 |
|------|------|----------|
| 渲染时 shader 未加载 | 中 | 渲染前检查，未加载时阻塞等待 |
| 后台加载失败 | 低 | 失败时下次渲染重试 |
| 内存碎片 | 低 | 使用 Shader 缓存池 |

## 实施步骤

1. **创建 TieredShaderManager** 结构体
2. **修改 Vello Renderer** 支持延迟加载接口
3. **在 dyxel 集成** 后台预加载线程
4. **添加降级策略** 未加载完成时的处理
