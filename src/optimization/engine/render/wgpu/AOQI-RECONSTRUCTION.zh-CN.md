# 渲染部分的实际接入

这是在官方 Ruffle nightly-2026-10-17 基线上自主重建的渲染集成，不是原私有 Rust 源码的逐行恢复。

## 位图放大

`shaders/bitmap.wgsl`、`shaders/common.wgsl` 从原 EXE 已验证的嵌入文本原样复制，分别对应恢复材料 `bitmap-fsr-easu.wgsl`、`common.wgsl`。`src/shaders.rs` 的真实 bitmap 管线继续使用这两个文件；并非未接入的示例。AMD FSR 许可从原发布版第三方声明保存为 `LICENSE-AMD-FSR.txt`。

CPU端 `GlobalsUniform` 增加16字节 `bitmap_upscale`，结构大小80字节；bind group 的最小大小随真实Rust类型同步。`TexturePool::get_globals` 把模式传给 `Globals::new`，vertex shader再以flat varying传给fragment。模式改变时清除Globals缓存，避免旧uniform继续生效。清空纹理池/调整窗口大小不会丢失模式配置。

`WgpuRenderBackend::new`读取以下**新重建的配置约定**，已有desktop入口无需另接分支：

| 环境变量 `AOQI_BITMAP_UPSCALE` | 行为 |
|---|---|
| 未设置、`off`、`0`或非法值 | 默认原采样行为 |
| `pixel`、`pixel-art`、`1` | 小位图精确像素保护 |
| `easu`、`2` | EASU放大加小位图精确像素保护 |

这些名字和默认选择不是原宿主配置恢复。宿主也可以调用公开的 `set_bitmap_upscale(BitmapUpscale)`，模式类型位于 `aoqi` 模块。

实际shader仅在横纵都检测到放大时启用增强。小位图条件为宽高都不超过96、无旋转/斜切、横纵分别为2/3/4倍整数放大；满足时采样源texel中心。其余符合放大条件的位图仅在EASU模式使用12-tap滤波。原始预乘alpha及颜色变换保留。默认Off不切换颜色算法，但加入的着色器逻辑有额外指令开销，未做性能测量。

## GPU闲置纹理预算

真正的 `TexturePool` 原本会无限保留出现过的尺寸桶。现按提交帧记录桶最后使用时间，`submit_frame` 提交后调用 `end_frame`：先从最老的桶开始，移除达到闲置帧数或超过闲置预算的桶。字节估算包含宽、高、图层数、MSAA采样数、像素格式和池中**已归还**的纹理数量。

以下前两项使用EXE中出现过的变量名称；新增的读回及缓存预算名称、所有默认值与算法均是本次重建设计：

| 环境变量 | 默认 | 边界 |
|---|---:|---|
| `AOQI_POOL_IDLE_BUDGET_MB` | 96 MiB | 0不保留有字节占用的闲置桶；非法值/乘法溢出回退默认 |
| `AOQI_POOL_RECENT_FRAMES` | 120帧 | 0每帧移除桶；非法值回退默认 |
| `AOQI_READBACK_IDLE_BUDGET_MB` | 64 MiB | 0不保留读回池闲置缓冲；非法值/溢出回退默认 |
| `AOQI_BITMAP_CACHE_BUDGET_MB` | 384 MiB | 0拒绝可再生成的 cacheAsBitmap 新分配；不限制真实 BitmapData |

公开 `set_texture_pool_limits(TexturePoolLimits)` 可供宿主运行时调整。未知纹理格式按每texel16字节保守估算；这不是显卡驱动真实显存占用，也不限制活跃纹理、mesh、Stage3D或游戏BitmapData的总内存。估算预算作用于桶整体，因此可能多释放一些，不能解释为精确LRU按单纹理淘汰。

淘汰只是移除池对可复用资源的所有权，不调用 `Texture::destroy`。被绘制流程持有的 `PoolEntry` 继续拥有纹理；池失效后它们正常析构，不再回到已经淘汰的池。GPU命令已提交的资源仍由wgpu的引用管理保护。流畅度修复版将主渲染和 offscreen 的尺寸桶、帧计数和闲置预算合并共享，取消每提交帧清空 offscreen 池；提交帧仍执行过期淘汰。viewport uniform使用相同闲置帧窗口，仍在使用中的Arc保留。

主渲染和离屏临时纹理池现在共享同一个闲置字节预算。资源归还时立即预留预算，超过预算即释放该包装对象，不能通过同帧多个尺寸桶或帧外 SyncHandle 归还绕过上限。读回缓冲按实际分配尺寸记账，复用大缓冲满足小请求不会缩小记账尺寸；归还时移除最大的闲置项直到容纳，单项超预算则直接释放。所有淘汰均保留仍在使用的对象。

临时纹理请求未命中且需要空间时，按尺寸桶最后访问顺序，清理较久未使用的闲置项，为当前请求留下可归还的空间。命中或预算充足时不排序淘汰。桶内尚未归还的资源不参与淘汰；工作集超过容量时仍可能拒留，不能保证所有场景均命中缓存。共享纹理的内容仍由各渲染通道正常清除、复制或绘制初始化，实时画面正确性需手动验收。

`create_bitmap_cache_texture` 与普通 `create_empty_texture` 分离：只为可重新生成的显示缓存设预算，超预算返回 `CacheBudgetExceeded`，由 core 退回正常绘制。native 路径还通过 wgpu OutOfMemory error scope 捕获该可选纹理创建失败，返回 `CacheAllocationFailed`，不提交无效纹理。这不是所有 GPU 分配的 OOM 恢复，真实 BitmapData、临时绘制目标及 swapchain 仍可能因系统资源耗尽失败。

尺寸改变时，先提交尚未提交的命令，再释放两个纹理池及读回池的闲置所有权，native 等待该提交完成后才调整目标。最多等待 2 秒；失败则告警并保留旧目标。没有对仍在使用的 BitmapData 调用 destroy。GPU 内存是否即时归还仍由 wgpu/驱动决定。

## 资源分类统计

`RenderBackend::resource_stats()` 返回上传位图、离屏 BitmapData、可再生成位图缓存、临时纹理、读回池五类统计。每类含 live 数量/描述符估算字节、数量/字节峰值、累计创建/释放数量及字节。不可复制的 RAII 计数令牌跟随真正资源包装对象，复用和 Arc 克隆不产生虚假创建；最终包装析构时更新释放，未分配成功不增加 live。预算拒绝和实际缓存分配失败分别累计次数及请求估算字节。

流畅度修复版另记录每类创建路径的计时次数、CPU 总耗时与最大耗时，以及临时池命中、未命中、压力淘汰和归还拒留次数。创建耗时包括路径内的 CPU 准备及提交，不能解释为 GPU 已完成耗时。桌面日志的 schema 2 与区间 FPS/耗时定义见 `../../../PERFORMANCE-TEST.zh-CN.md`。

这些是 **Rust 包装对象所有权统计，不是驱动 GPU 驻留量或整个进程总 GPU 内存**。已提交命令、texture view、bind group 等 wgpu 引用可能晚于包装析构释放实际资源。统计不涵盖 mesh、Stage3D、staging belt、swapchain、独立 exporter 截图目标；临时纹理含主/离屏池和 PixelBender 临时纹理，读回类专指渲染器的读回池。预算只限制相应缓存，不能保证整个进程内存上限。

## 验证与未完成范围

`cargo test -p ruffle_render_wgpu --lib` 包含新增设置解析、预算/过期/存活对象生命周期，以及实际common shader拼接的Naga解析和语义验证。14份shader分别以1和64个transform拼接，共28种组合，同时核对uniform大小和偏移。测试不需要GPU窗口或游戏联网。新增 CPU 测试覆盖跨桶共享预算、归还即时淘汰、超大项拒留、复用时保留实际尺寸、清池不影响在用项、最终 Drop 计数、缓存拒绝/失败不虚增 live；不等同于 GPU OOM 压力实测。

真实GPU渲染、不同显卡、游戏画面、显存曲线、帧率均仍需运行验收。没有宣称全屏低分辨率FSR加速。原恢复的 `aoqi-stage-box-filter.wgsl` 尚未接入舞台超采样，mesh arena、驱动显存驻留控制和全局 OOM 恢复不是本改造已完成的内容。
