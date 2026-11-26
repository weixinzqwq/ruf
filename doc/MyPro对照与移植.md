# MyPro 优化对照与验收

2026-11-23。两份合作方报告的文件内容逐字节相同。以下 E 编号均指该报告。实现基于本项目的 `src/optimization`、本地官方游戏素材和现有运行包。性能实验沿用 11 月 23 日 Flux 包的画质、帧率和立绘设置。[报告](../MyPro_v1.0.0_static_analysis.txt) · [spec 文件](../MyPro_v1.0.0_static_analysis_spec.txt)

## 运行与单项对照

11 月 24 日进一步核对根目录的 MyPro **v1.0.1**。发行 EXE 文件偏移 `0x1f57301` 有配置名 `AOQI_FUSE_GLOW`，`0x1f4a9ac` 的内嵌着色器说明将最后一次纵向模糊与 Glow 合成，`0x1f4b3b9` 附近有恢复八位精度的运算。另一处 `0x1f4cf53` 描述同时融合横纵模糊。v2 的 PID 67460 在最慢的 10.64 秒小窗样本中有 4,386 次 Glow 调用，渲染线程为这一类滤镜记录了 806.55 ms。此次项目候选先验证最后一次纵向模糊的融合。[合作方 E07](../MyPro_v1.0.0_static_analysis.txt) · [v2 实战窗口](evidence/mypro-runtime-20261124/live-142192.json) · [候选与计数](goto与子树遍历.md#11-月-24-日-mypro-战斗跳帧候选)

后续 v1.0.1 报告专门分析多场战斗后的衰减。长时包沿用本页的大 CWS、位图组和前版背包策略，提供独立的长时维护对照入口。[v1.0.1 各项取舍与结果](MyPro长时战斗优化.md)

后续[背包切换包](背包详情与SWF去重.md#11-月-23-日背包切换修复)沿用本轮 MyPro 开关，并为背包 SWF 提供独立的原版对照入口。

| 入口 | 与常规入口相比，改变的行为 |
| --- | --- |
| [常规游戏](../dist/aola-mypro-20261123/run-game.cmd) | 大 CWS 后台解压、符合条件的位图组直接绘制；播放原有入场背景转场 |
| [CWS 前台解压](../dist/aola-mypro-20261123/run-foreground-cws.cmd) | 只将大 CWS 调回调用线程；ZWS 仍在后台 |
| [位图组原路径](../dist/aola-mypro-20261123/run-isolated-bitmap-groups.cmd) | 只恢复多位图组的中间表面；单位图原有快速路径仍然有效 |
| [跳过额外战斗背景转场](../dist/aola-mypro-20261123/run-skip-battle-backdrop.cmd) | 只跳过符合条件的 GQ／星辉背景转场 |

每个入口启动同一 EXE，使用包内同一个 `aola-data`。原有公告、立绘、万域之潮及 ZWS 对照入口继续随包提供。新包首次使用时手动登录。实机对比请固定窗口、地图、亚比和操作顺序，分别记录首次载入及同一资源再次载入。

## 11 月 24 日发光滤镜融合

`AOLA_FUSE_GLOW=1` 把最后一次纵向模糊放进发光合成的着色器。共用的模糊采样函数在合成前按八位通道精度取整，仍使用两张轮换的中间纹理。整张单采样 RGBA8 纹理、双轴模糊均大于 1 且无偏移时，每次命中省去一次单独绘制。局部裁剪、带偏移的阴影及单轴模糊沿用原来的渲染路径。`performanceInterval.filterSetup.fusedGlowPasses` 只计实际命中，`optimizationMode.fuseGlowRequested` 记录启动设置。该开关适用于所有符合条件的绘制。[Glow 实现](../src/optimization/engine/render/wgpu/src/filters/glow.rs) · [共用采样函数](../src/optimization/engine/render/wgpu/shaders/filter/blur_kernel.wgsl)

在本机 DX12 上预热两套管线，同一设备上交替关闭和开启融合。下表为每组四次计时的中位数，单位为六帧的毫秒数，含提交以及每帧等待 GPU 完成。每轮两种设置的最终画面逐字节相同。

| 离线负载 | 六帧省下的绘制次数 | 第一轮 关 → 开 | 第二轮 关 → 开 |
| --- | ---: | ---: | ---: |
| 每帧 240 次小纹理发光，参数频繁变化 | 1,440 | 1,855.8 → 1,466.3 ms | 1,288.3 → 1,477.3 ms |
| 每帧 78 次混合尺寸发光 | 468 | 148.5 → 140.8 ms | 342.4 → 305.2 ms |

默认入口关闭融合，小纹理组的耗时方向在复测时发生反转。[两项同开的入口](../dist/aola-battle-render-20261124/run-goto-and-glow.cmd)相对默认入口只增加发光融合，可用相同技能的 `fusedGlowPasses`、总渲染耗时和呈现 FPS 评估实际收益。MyPro 原版日志未记录该着色器的战斗命中数。[逐次计时与验收记录](evidence/mypro-runtime-20261124/battle-render-candidate.json)

发行 EXE 另运行了一份可见的带发光 Sprite 动画。影片进入已识别战斗状态，每个普通帧执行 16 次跨帧跳转。取两侧各约 5 秒的完整采样，均完成了 251 个普通帧、4,000 次局部跳帧。关闭融合时呈现 251 帧，246 次发光走了 492 次独立模糊绘制；开启时呈现 247 帧，247 次发光命中融合，剩余 247 次独立模糊绘制。这也验证了当前播放器在同一个普通帧中反复跳帧时，实际呈现次数远小于跳帧次数。[同 PID 性能日志与区间计数](../build/mypro-battle-render-20261124/desktop-glow-summary.json)

独立的 DX12 回读在关闭、开启融合时分别对照了 v2 的 32 张参考图像，最大通道差值均为 0。相同 AS3 影片又在两个发行播放器中各读回 2,304 个像素，覆盖外发光、内发光和 knockout。每个播放器自身开关两态均逐像素相同。跨播放器比较时，knockout 场景有两个像素存在差异，最大通道差为 2，两侧开关关闭时也有这两处差异。[回读与原始日志索引](evidence/mypro-runtime-20261124/battle-render-candidate.json)

11 月 25 日使用现有 DX12 渲染测试重新运行 `fused_glow_sizes_on_dx12`。同设备预热后交替开关，每组四次，最终画面逐像素一致。六帧小纹理组少画 1,440 次，等到 GPU 完成的时间中位数为关闭 1,619.75、开启 1,695.79 毫秒；混合尺寸组少画 468 次，两侧为 244.53、244.69 毫秒。本轮两组都没有显示耗时收益，实际对局需结合命中数和呈现帧率验收。[DX12 测试](../src/optimization/engine/render/wgpu/src/filter_submit_tests.rs) · [战中换亚比的渲染低谷](goto与子树遍历.md#战中换亚比的渲染低谷)

11 月 25 日实战 PID 129172 使用同包的发光融合加战中 GC 入口，完整战中区间融合计数增加 41,477 次。第二场发光密度接近的窗口中，独立模糊绘制从上次 54.96 降到本次 34.59～35.36 次/呈现，临时纹理新建仍为 42.37～45.60 张/呈现。[实战窗口与耗时](goto与子树遍历.md#发光融合开启后的三场实战)

## 大 CWS 解压

在实验包的 `AOQI_STAGE3_MODE=sleep` 下，`AOLA_BACKGROUND_CWS=1` 让根电影、URL 加载的电影和导入电影，将以 `CWS` 开头、压缩输入至少 16 KiB 的解码工作交给现有两个资源线程。结果准备好后才交付加载器。小文件保留原路径；这个开关也保持了 AS3 `Loader.loadBytes` 的原有同步行为和错误顺序。游戏中经过 `URLLoader → ByteArray → Loader.loadBytes` 的 CWS 因而不会仅靠此项转到后台。[调度判定](../src/optimization/engine/core/common/src/aoqi_work.rs) · [电影加载入口](../src/optimization/engine/core/src/loader.rs)

在 `performance-*.jsonl` 中，`myProPortBuild=aola-mypro-v1` 标识本轮代码，`optimizationMode.backgroundLargeCws` 表示调度策略，`processResourceWorker.completedLargeCws` 是进程累计完成的合格后台作业数，包含解码返回错误的作业。比较前后两次采样的增量可以判断场景是否走到这条路径。

本机对 `loader`、`library_util`、`library_common`、`play`、`library_interfaces` 五个归档 CWS 分别开关重跑，压缩输入为 72,311 至 1,118,994 字节。开启后五个均在 `aoqi-resource-0/1` 解码，关闭后均在调用线程。两组都核对了原始响应、SWF 头、解压标签和损坏版本的错误。以上证明工作移出了调用线程；下载至可见首帧的总时间仍须实机测量。[五个样本](evidence/mypro-20261123/summary.json) · [16 KiB 门槛与开关隔离](evidence/mypro-20261123/resource-mode.txt)

## LoaderInfo.bytes 字节写入

本地游戏加载器的 `DomainLoader.onLoadByteComplete` 在非调试分支中循环 9 次，每次读取 `LoaderInfo.bytes` 并修改返回数组的一个字节。本引擎每次构造新的 FWS 字节数组。此前增长数组会先把新增区域全部置零，随即又用 SWF 数据覆盖。本轮由 `ByteArrayStorage::write_at` 直接追加已有源数据，只为写入偏移前的空洞补零。每次 getter 都保留完整内容、独立可写性、原来的位置和字节序。历史实机采样中，该回调有一条记录包含 3 次慢调用，最长 473.8894 ms；该数值包含嵌套回调，无法归为拷贝耗时。[Flash getter](../src/optimization/engine/core/src/avm2/globals/flash/display/loader_info.rs) · [底层写入](../src/optimization/engine/core/src/avm2/bytearray.rs) · [历史采样](evidence/swf-lifecycle/switch-stall-20261121/response-analysis.json)

底层写入与旧实现对照了 48 组边界和 2 组越界错误。AVM2 的 ByteArray、LoaderInfo、Stage LoaderInfo 及 `loadBytes` 回归在 CWS 开、关下各通过 26 项；每组原套件都将 `bytearray_oom` 标为忽略。包含逐字节读取自身 SWF 的 `loaderinfo_quine`。[写入边界](evidence/mypro-20261123/bytearray-write.txt) · [ByteArray](evidence/mypro-20261123/bytearray-1.txt) · [LoaderInfo](evidence/mypro-20261123/loaderinfo-1.txt) · [loadBytes](evidence/mypro-20261123/loader_loadbytes-1.txt)

编译结束后，用同一 release 测试二进制对照原写法与新写法。每批 9 份独立 ByteArray，各自从 getter 的 FWS 头开始写入标签。预热一对后交替运行 12 对，每份结果完整逐字节核对。表内时间为一批 9 次标签写入的中位数，解压、结果核对、AS3 解释与显示均在计时范围之外。

| 本地官方素材 | 每批少写的零字节 | 原写法 | 新写法 |
| --- | ---: | ---: | ---: |
| [背包 2026111953323155](evidence/mypro-20261123/write-backpack.txt) | 19,777,698 | 4.32 ms | 2.53 ms |
| [接口库 2026102936286521](evidence/mypro-20261123/write-interfaces.txt) | 28,045,494 | 7.10 ms | 4.03 ms |
| [战斗 261112849126071](evidence/mypro-20261123/write-battle.txt) | 59,163,138 | 16.07 ms | 9.77 ms |

这些字节表示减少的内存写入量。返回数组的长度和内容保持一致，进程内存峰值及点击到新模型首帧的耗时需另行测量。

## 多位图直接绘制

已有引擎可以裁剪局部 blend 的中间画布，也可直接绘制部分单位图。本轮处理 2–64 条纯位图命令组成的组。渲染器要求固定功能混合、无遮罩、无额外颜色或透视变换、单位缩放、非负整数平移，并检查每张位图的目标范围与此前所有位图的外包框互不相交。边缘贴合可以命中。重叠、落在外包框内部的空隙或遇到其他不符合条件的命令，均走原有组隔离。诊断字段 `performanceInterval.renderCounts.directBitmapGroups` 是采样区间命中的组数。[渲染实现](../src/optimization/engine/render/wgpu/src/surface/commands.rs)

Intel Iris Xe 的 DX12 离屏回读覆盖 Low／High、6 种混合模式和分离、贴边、重叠、小数平移、遮罩五种布局。开启及关闭开关各对照 60 组，最大通道差值均为 0。开启后命中 20 组，合计少 20 个中间表面；关闭后两个计数均为 0。实际战斗的命中比例和帧耗时需要游戏日志。[开启记录](evidence/mypro-20261123/bitmap-groups-1.txt) · [关闭记录](evidence/mypro-20261123/bitmap-groups-0.txt) · [原单位图回归](evidence/mypro-20261123/bitmap-single.txt)

## 额外入场背景转场

当前本地官方战斗原件为 `petfightnew~261112849126071.swf`。普通战斗的 `StartMovie.enterFight` 先调用 `dispose()`，随后优先播放 GQ 背景，有星辉且无 GQ 时播放星辉背景。`StartMovie3v3.enterFight` 只检查星辉。`AOLA_SKIP_BATTLE_BACKDROP=1` 在三个背景请求点跳到原方法已有的完成闭包，由它创建并挂接 `FightPanel`、发送一次 `FIGHT_UI_START`。更早的开场动画仍会播放。常规入口将这个开关设为 0。[补丁生成](../src/tools/swf-patches/battle_patch.java)

在同一官方原件上重建后，相对于上一轮立绘候选，5,620 个 ABC 方法体中仅两个 `enterFight` 的指令发生变化。AVM2 夹具执行了两个实际方法及完成闭包，以可观察的游戏依赖测试普通／3v3、开放／跳过、无标记／GQ／星辉／两者兼有共 16 个场景。检查包含停止计时器与移除入场对象、原路径等待动画完成、跳过时直接推进，以及面板挂接与事件各发生一次。相同候选也通过了原有万域之潮和 32 个立绘场景。[方法比较](evidence/mypro-20261123/battle-methods.txt) · [行为夹具](../src/tools/swf-patches/harness/BattleBackdropProbe.as) · [转场回归](evidence/mypro-20261123/avm2-backdrop.txt) · [立绘回归](evidence/mypro-20261123/avm2-portraits.txt) · [万域回归](evidence/mypro-20261123/avm2-wanyu.txt)

实机查看 `startup-*.jsonl`。先确认 `type=battle-swf` 且 `result=hit`，再看 `type=battle-effect` 和 `battle-backdrop-gq` 或 `battle-backdrop-xinghui`。后者每种通道仅在首次实际跳过时记录。另需在普通和 3v3 分别确认面板可操作、技能按钮出现及下一回合推进。官方版本或完整响应内容变化时会沿用收到的原资源。

## 报告中各项的取舍

| 报告项 | 当前项目的判断与处理 |
| --- | --- |
| E01、E14、E15 基线与测量 | 合作方提供的是样本静态分析。保留本项目逻辑帧、渲染、资源生命周期与回调计时；本轮新增 CWS 和多位图命中数。通用 GPU 渲染、指令预解码沿用已有引擎能力。 |
| E02 AS3 原生热点 | 本项目 8 次背包详情共计 12,924.55 ms，方法表中可对应到报告的 11 个方法累计自身耗时 130.45 ms。其中 `splitToIntArray` 调用 6,341 次，占 34.48 ms。最慢一次详情耗时 2,437.79 ms，这些方法合计 15.56 ms。优先保留脚本兼容性。相同长度原地换项、重复 ID 的优先顺序仍需在做称号索引前覆盖。[两组已归档实机样本及逐项计数](evidence/mypro-20261123/e02-existing-profile.json) |
| E03、E04 加载路径 | 已接入上述字节写入和 CWS 调度。引擎已有可选弱引用 SWF 解压数据去重，`dedup`／`all` 模式启用；本包的 `sleep` 模式未启用。没有额外保留 48 份强引用。 |
| E05 磁盘缓存 | 本包共用已有公开 HTTP 响应缓存，保留原压缩体。新增解压后的磁盘副本和 mmap 可能改善跨进程热启动，同时引入额外写盘、缺页和磁盘占用。待同资源跨进程重复解压数据支持后再引入第二层缓存。 |
| E06、E07 混合与滤镜 | 11 月 23 日包接入多位图路径，Glow 复用 blur 的闲置目标，最终发光合成单独绘制。11 月 24 日的战斗包另提供可选的[最后一次纵向模糊与发光融合](#11-月-24-日发光滤镜融合)，覆盖精度和裁剪对照。 |
| E08 空闲纹理和网格 | 已有显示位图预算、共享临时纹理池和可重建网格／位图的闲置回收。在本地压力测试中 96 MiB 出现 120 次热纹理重建，256 MiB 为 0，保留本项目 256 MiB 临时池预算。将闲置阈值缩到六／八秒后的重建成本待验证。 |
| E09 库、GC 和工作集 | 已有电影持有符号库、全局弱索引、分片 GC 及完整周期后的盘点。强制 GC 或清工作集可能增加停顿或后续缺页，本轮沿用已验证的资源策略。长期效果看同地图首尾的对象、私有内存及重建计数。 |
| E10、E11 战斗表现 | 五类变身立绘和万域攻击已有独立行为验证。本轮将额外入场背景转场做成独立选择。换宠、逃跑、受击、死亡、神兵、战斗图层及全局技能屏蔽牵涉其他回调或可见信息，未并入常规启动。将神兵升级替换为空 SWF 还需要核对当前调用方如何判断完成。 |
| E12、E13 宿主及版本差异 | WebView 列表分页、H5／Spine、FSR 属不同宿主或内容路径；强制 60 FPS 会改变按帧倒计时的节奏。沿用项目原有设置，保留已经验证的公告和战斗退出清理。 |

已有策略与测试详见[回收与纹理复用](回收与纹理复用.md)、[资源库与长时稳定性](资源库与长时稳定性.md)及[AolaFlux 逐项对照](AolaFlux对照与移植.md)。本地原件和离线工具已足够重建本轮改动。

## 运行包验收

本轮 Windows release EXE 与编译产物逐字节一致，`--help` 包含项目的宿主和数据目录参数。两个随包原件与项目素材一致。9 个启动脚本分别在带空格的路径用默认与自定义磁盘预算执行，共 18 次。使用参数记录程序核对开关、帧率、数据目录与退出码。[发行包与入口记录](evidence/mypro-20261123/package.json)

桌面侧 11 项项目回归通过，其中包括万域攻击、五类立绘和两种背景通道的 8 种宿主开关组合。诊断回归另外核对了区间多位图组计数，避免将以前采样段的命中混到本段。[宿主](evidence/mypro-20261123/desktop-host.txt) · [区间计数](evidence/mypro-20261123/desktop-diagnostics.txt)

实际 navigator 在本地公开缓存上执行了关闭、开启、再关闭的资源获取。只有官方版本及完整内容匹配时才返回与独立重建相同的战斗候选；老版本和内容不符的响应均返回原数据。此项验收关闭了网络客户端。[资源加载回归](evidence/mypro-20261123/battle-fetch.txt)

引擎 `all` 模式通过 189 项核心测试。包内 `sleep` 模式通过 188 项常规测试，另有一项老测试写死了“通用任务必须在后台”；该断言适用 `all`，而 `sleep` 本来就把通用任务放在调用线程。大 CWS 的两种模式另有专门测试。渲染常规测试通过 36 项，DX12 回读另行显式运行。[完整模式](evidence/mypro-20261123/core-all.txt) · [实验包模式](evidence/mypro-20261123/core-sleep.txt) · [渲染测试](evidence/mypro-20261123/renderer-unit.txt)
