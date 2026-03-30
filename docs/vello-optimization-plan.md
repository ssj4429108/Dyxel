# Vello 启动优化实施方案

## 背景分析

Vello 0.7 的 shader 结构：
- `fine.wgsl`: 1286 行 - 最终像素渲染（最大瓶颈）
- `blend.wgsl`: 200+ 行 - 15 种混合模式
- `flatten.wgsl`: 923 行 - 路径扁平化
- `coarse.wgsl`: 469 行 - 粗粒度光栅化

首次启动时，驱动需要将这些 WGSL 编译为 GPU ISA，耗时约 800-1200ms。

---

## 第一阶段：Essential-Kernel（核心路径裁剪）

### 目标
将首个 ComputePipeline 创建时间从 800ms 压低到 200-300ms。

### 技术方案

#### 1. 创建 Essential Shader 变体

在 `vello_shaders` 中新增 `fine_essential.wgsl`，通过条件编译移除非核心功能：

```wgsl
// Essential 模式只保留 SourceOver 混合
#ifdef ESSENTIAL_MODE
    // 简化版 blend，只支持 SourceOver
    fn blend_simple(src: vec4<f32>, dst: vec4<f32>) -> vec4<f32> {
        return src + dst * (1.0 - src.a);
    }
#else
    // 完整版 blend（原有实现）
    #import blend
#endif
```

#### 2. 裁剪清单

| 功能 | Essential | Full | 节省估算 |
|------|-----------|------|----------|
| 混合模式 | SourceOver only | 15 modes | ~40% |
| 高斯模糊 | ❌ 移除 | ✅ 支持 | ~15% |
| 复杂渐变 | 线性 only | 线性/径向/圆锥 | ~10% |
| 极细描边 | ❌ 移除 | ✅ 支持 | ~5% |
| MSAA 16x | ❌ 移除 | ✅ 支持 | ~20% |

#### 3. 代码实施步骤

**Step 1: 修改 vello_shaders/build.rs**
```rust
// 添加 essential 变体编译
let essential_shaders = compile_shaders_with_flag("ESSENTIAL_MODE");
```

**Step 2: 新增 fine_essential.wgsl**
- 基于 fine.wgsl 复制
- 移除 `#import blend`，内联简化版 blend
- 移除 MSAA 16x 支持（仅保留 8x）
- 移除模糊效果处理分支

**Step 3: 修改 vello/src/shaders.rs**
```rust
pub struct EssentialShaders {
    pub pathtag_reduce: ShaderId,
    pub flatten: ShaderId,
    // ... 只包含核心路径
    pub fine_area: ShaderId,  // 使用简化版 shader
}

pub(crate) fn essential_shaders(...) -> Result<EssentialShaders, Error> {
    // 加载 essential shader 变体
}
```

**Step 4: Renderer 集成**
```rust
pub enum ShaderMode {
    Essential,  // 快速启动
    Full,       // 完整功能
}

impl Renderer {
    pub fn new_with_mode(device, mode: ShaderMode) -> Self {
        match mode {
            ShaderMode::Essential => load_essential_shaders(),
            ShaderMode::Full => load_full_shaders(),
        }
    }
}
```

---

## 第二阶段：Specialization Constants（硬件级死代码删除）

### 目标
利用 GPU 驱动的常量折叠，在编译时删除不符合条件的代码分支。

### 技术方案

#### 1. 可特化常量定义

```wgsl
// shared/config.wgsl
#ifdef USE_SPECIALIZATION
@id(0) override MAX_ROUGHNESS: f32 = 1.0;
@id(1) override BIN_SIZE: u32 = 256u;
@id(2) override TILE_SIZE: u32 = 16u;
@id(3) override ENABLE_BLUR: bool = true;
@id(4) override ENABLE_COMPLEX_GRADIENT: bool = true;
#else
const MAX_ROUGHNESS: f32 = 1.0;
const BIN_SIZE: u32 = 256u;
const TILE_SIZE: u32 = 16u;
const ENABLE_BLUR: bool = true;
const ENABLE_COMPLEX_GRADIENT: bool = true;
#endif
```

#### 2. 代码中的条件分支

```wgsl
// fine.wgsl 中的使用
if ENABLE_BLUR && roughness > MAX_ROUGHNESS {
    // 复杂模糊处理
} else {
    // 简化路径
}

if ENABLE_COMPLEX_GRADIENT {
    // 径向/圆锥渐变
} else {
    // 仅线性渐变
}
```

#### 3. Rust 层配置

```rust
// wgpu pipeline 创建时传入特化常量
let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
    label: Some("vello_fine_essential"),
    layout: Some(&layout),
    module: &shader_module,
    entry_point: "main",
    // wgpu 未来版本支持
    constants: &[
        (0, 0.5.into()),           // MAX_ROUGHNESS
        (1, 128u32.into()),        // BIN_SIZE
        (2, 8u32.into()),          // TILE_SIZE
        (3, false.into()),         // ENABLE_BLUR
        (4, false.into()),         // ENABLE_COMPLEX_GRADIENT
    ],
});
```

---

## 实施路线图

### Week 1: Essential Kernel 基础
- [ ] Fork vello_shaders 到本地
- [ ] 创建 fine_essential.wgsl（移除 blend/模糊/复杂渐变）
- [ ] 修改 build.rs 支持双变体编译
- [ ] 测试编译后 shader 体积

### Week 2: Renderer 集成
- [ ] 修改 vello/src/shaders.rs 加载 essential shaders
- [ ] 新增 RendererOptions.shader_mode 选项
- [ ] 在 dyxel 中集成：LowEnd -> Essential, HighEnd -> Full
- [ ] 测试启动时间差异

### Week 3: Specialization Constants
- [ ] 调研 wgpu 最新版本对 specialization constants 的支持
- [ ] 修改 config.wgsl 添加 override 定义
- [ ] 实现 ShaderConstants 结构体传递
- [ ] 在 LowEnd 设备上测试死代码消除效果

### Week 4: 验证与调优
- [ ] Android LowEnd 设备实测（启动时间、内存、FPS）
- [ ] 回归测试：确保渲染结果正确
- [ ] 性能对比报告

---

## 预期效果

| 指标 | 当前 | Phase 1 | Phase 2 |
|------|------|---------|---------|
| 首个 Pipeline | 800ms | 250ms | 150ms |
| 全部 Shaders | 1800ms | 600ms | 400ms |
| 内存占用 | 690MB | 450MB | 400MB |
| 低端机 FPS | 20-30 | 45-55 | 50-60 |

---

## 风险评估

| 风险 | 概率 | 影响 | 缓解措施 |
|------|------|------|----------|
| 渲染结果差异 | 中 | 高 | 建立像素级对比测试 |
| wgpu API 变更 | 低 | 中 | 锁定 wgpu 版本 |
| 维护成本 | 中 | 中 | 脚本自动化 shader 生成 |
