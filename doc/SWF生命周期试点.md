# 背包视图切换的 SWF 生命周期试点

## 目标与资源

试点观察背包从普通视图切换到传奇视图后，用户在位移动画结束前关闭背包，旧控件是否继续移动。最初在 2026-11-06 的归档原件上复现；2026-11-21 登录采集取得了新版官方背包。

## 11 月 21 日实机采集

本次 `capture-21269-6015` 正常退出，两份 SWF 各收到一次完整网络响应。以下时间相对于启动日志起点。

| 资源 | 首次实收 | 版本 | 大小 | 与首版试点的关系 |
| --- | ---: | --- | ---: | --- |
| `library/util` | 21.232 秒 | `2026090340574444` | 158,135 字节 | 与归档完整内容一致，保留原 TweenLite 依赖 |
| `pet/newpetpackage` | 42.565 秒 | `2026111953323155` | 1,525,161 字节 | 背包已换版，按本次实收原件重新构建 |

原件已归档到 `resources/swf-reference/lifecycle/2026111953323155/newpetpackage~2026111953323155.swf`。与历史背包逐 tag 对比，舞台头、3,357 个顶层 tag 的类型及顺序相同，官方更新改变了其中唯一的 DoABC。性能日志含 9 次背包详情响应和结束记录，首末跨度约 94 秒，期间记录到 4 种物理窗口尺寸。这个会话可确认官方资源版本，尚未加载功能候选，也缺少关闭时旧控件坐标的游戏内计数。[原始采集结果](../dist/aola-swf-lifecycle-20261121/runs/capture-21269-6015/resource-load.json)

第二个背包详情响应的采样中出现一次 `PackageSkinManager.changeClassify()`。初始化和用户点击都可能进入这个方法。v1 没有记录对应的关闭和复开，因此无法从这次采集还原一次完整操作。9 次详情响应均有记录，日志中有一次未处理的外部桥调用，资源本身正常取得。[脱敏运行摘要](evidence/swf-lifecycle/live-capture-20261121/summary.json)

## 当前版本的适配

从新原件重新导出的 16 个背包类、3 个补间库类与上轮导出一致。13 个目标方法的原始 P-code 内容一致，但四个方向或位置相关方法的 body index 发生变化。下表记录结束动画使用的方法编号，编号以相应版本的 DoABC 为准。[方法复核](../build/swf-patches/pet-skin-lifecycle/current-preflight/class-comparison.json)

| `disappear()` 的实现 | 历史版 body | 本次实收版 body |
| --- | ---: | ---: |
| `MotionLeft` | 281 | 267 |
| `MotionDown` | 272 | 272 |
| `MotionRight` | 286 | 277 |
| `MotionUp` | 267 | 282 |
| `MotionNormal` | 277 | 287 |

当前控制件为 1,525,214 字节，两次独立生成的压缩文件一致；解压后的全部 2,197,537 字节与本次实收原件一致。重新构建的功能候选为 1,525,329 字节，两次构建逐字节一致。13 个方法的修改重新通过本版 ABC 静态检查，其余 tag 与本次原件完全相同。旧版控制件和候选均不能用于这一版。[当前控制件报告](../build/swf-patches/roundtrip-pet-skin/2026111953323155/report.json) · [当前候选报告](../build/swf-patches/pet-skin-lifecycle/2026111953323155/report.json)

用同一本仓库 release 测试器，对当前原件先测无修改回写，再按 A/B、B/A、A/B 顺序各测一对。7 次运行全部通过。原件 90/90 次关闭后旧控件继续移动，功能候选 0/90 次。两组各 105 个外部动画都恰好完成一次并达到预期终点，五种位移动画的自然终点也一致。两组在正常清扫后保留的旧目标键都是 0。这里测量的是当前背包真实字节码和替代面板素材的 Sprite，游戏内画面、点击与长期帧时间另行验收。[当前版逐轮证据](evidence/swf-lifecycle/harness-series-2026111953323155-20261121T094610831282Z/sequence.json)

新版加载器用两份实收原件和独立重建的两个对照文件完成 15 项专项回归。旧版 URL 即使返回新版的字节也会回退。原件、回写和候选三组都要在同一播放器中先确认 util、再确认背包的完整响应，才记录本组命中。功能组还会逐字节检查候选的 42 处许可改动及所有其余区间。[三组加载验收](evidence/swf-lifecycle/loader-current/equal-dependency.log) · [两次独立构建](../build/swf-patches/pet-skin-lifecycle/2026111953323155/reproducibility.json)

桌面宿主只记录四种成功返回的动作。在当前原件、回写版和候选各 1,509 个方法中，分别定位了同样的 6 个相关方法。只有 `onClickMe → changeStyle → changeClassify` 才计一次点击切换。只有面板 `disposeEvent` 越过重复关闭守卫、调用皮肤管理器并成功返回，才计一次清理。这个点位于面板的关闭通知之后、共享缓存清理之前。[方法与事件对照](evidence/swf-lifecycle/operations-2026111953323155/bindings.json)

在带新观测的 release 测试器中重跑原件，开关开启时记录了预期的 42 次分类返回，关闭时为 0。两次都通过了行为断言；Sprite 测试没有执行完整 UI，三种 UI 专属事件均为 0。主验收另用独立重建的功能候选复测，得到相同的 42 次分类返回和 0 次 UI 事件。[观测开关对照](evidence/swf-lifecycle/operation-headless-20261121T095239Z/operations.json)

### 当前版的实机操作

已核对分发包内的四份 SWF、五个启动入口和最终 release 程序。83 个诊断样例覆盖错版、错序、跨播放器复开及时间边界；另从实际分发的汇总入口运行了 12 个正反例。[最终包验收](evidence/swf-lifecycle/capture-current-20261121/package.json) · [诊断样例](evidence/swf-lifecycle/capture-current-20261121/summary-tests.json)

首次验证使用 v2 的功能候选 B。正常登录，进入固定地图并打开背包，选择带“点击查看海报形态”按钮的亚比。先切换一次并等动画自然结束，检查画面。随后再切换一次，在 1 秒内关掉背包，重新打开并检查图片、按钮及输入，然后正常退出游戏。每次启动都有独立运行目录。[启动功能候选 B](../dist/aola-swf-lifecycle-20261121-current/run-lifecycle.cmd) · [启动原件 A](../dist/aola-swf-lifecycle-20261121-current/run-original.cmd) · [退出后回退官方](../dist/aola-swf-lifecycle-20261121-current/run-official.cmd)

在 `runs/lifecycle-*/resource-load.json` 中，先看 `resource_load_verified`，再看 `operations.verified_resource_quick_reopens`。前者要求实收资源校验通过。后者大于 0，表示同一个播放器命中资源后出现了面板构造、点击切换、清理、再构造，且最后一次点击切换到清理的间隔不足 1 秒。计时取自方法成功返回。自动分类单独计数；若复开复用了旧面板，报告可保留切换和清理间隔，但不会猜测发生了一次新构造。复开后的画面和输入由操作者核对。正式短测和长测按下方固定口径执行。

### 第一次 v2 功能实测

`lifecycle-27130-9074` 正常结束，同一播放器先确认工具库，再通过网络命中 1,525,329 字节的背包功能候选。记录了 2 次面板构造、2 次点击切换和 1 次面板清理，形成 1 次有序复开。最后一次点击切换到面板清理间隔 12.702 秒，未覆盖 1 秒动画内提前关闭。玩家本次报告切换不同亚比时卡住 2～3 秒。8 次详情处理的时间和归因另记。[运行结果](../dist/aola-swf-lifecycle-20261121-current/runs/lifecycle-27130-9074/resource-load.json) · [亚比切换停顿](背包详情与SWF去重.md#切换亚比时的实机停顿)

## 历史试点材料

| 材料 | 位置 | 用途 |
| --- | --- | --- |
| 原版背包 `pet/newpetpackage~2026102936286521.swf` | `doc/archive/历史修改证据.zip` 内的 `evidence/pet-detail-20261106/source/pet_newpetpackage~2026102936286521.swf` | 固定离线原件，保留未修改脚本、时间轴和素材 |
| TweenLite 所在工具库 | `resources/original/library_util~2026090340574444.swf` | 验证实际补间的注册、更新与自然结束 |
| 原类的接口与父类 | `resources/original/library_interfaces~2026102936286521.swf`、`resources/original/library_common~2026102936286521.swf` | 为离线测试器提供真实的接口定义及 DialogBase |
| 背包补丁的历史加载记录 | [2026-11-06 背包实验](背包详情与SWF去重.md) | 追溯该历史版本以及当时的原件／候选模式 |

## 谁持有对象，在哪里结束

| 操作 | 调用链 | 观察内容 |
| --- | --- | --- |
| 开始切换 | `SimpleViewController.onClickMe` 的 `btnshowlegendview` 分支 → `changeStyle` → `PackageSkinManager.changeClassify` → `Style.appear/disappear` → 对应的 `Motion` | 每个样式的目标控件与由此次切换创建的位移补间 |
| 逐帧更新 | `TweenLite` 根时间线推进，`MotionUp.disappear` 等方法创建的补间写入目标坐标 | 同一控件退出后的坐标变化次数、最终坐标与自然结束时间 |
| 关闭背包 | `PetPackagePanel.close` 或 `dispose` → `disposeEvent` 的 `_isDispose` 守卫 → `PET_SERVICE_DISPOSE` → `on_pet_package_disposed` → `PackageSkinManager.dispose` | 原管理器清空单例引用。补间自身保留目标，旧目标是否继续更新需动态验证 |
| 再次打开 | 新面板取得新的管理器并重新注册样式 | 前一代的工作是否影响新面板，仍在场的其他控件是否继续正常更新 |

离线运行保留共享的 TweenLite 根时间线。单个面板可以持有自己的动画句柄；关停共享时间线或清空全部目标的动画会影响其他使用者。库会周期性清理已经结束的补间，短暂保留和持续增长需要分别度量。

## 预先固定的验收口径

1. 原件、无逻辑修改回写版与功能候选使用同一组输入。先观察自然结束，再在补间活跃时关闭，最后进行重复关闭和快速复开。每轮使用新进程或可验证的干净状态。
2. 原类隔离测试记录目标坐标和退出后仍活跃的补间。功能候选的目标是关闭后旧目标位置保持稳定，同场其他动画及新面板仍按原顺序到达各自终点。业务完成通知、输入恢复及最终画面另外在完整游戏中核对。
3. 完整游戏短测固定地图、角色、背包及皮肤、视图按钮、窗口像素、画质与倍率。先确认该资源实际命中，再预热 5 次、测量 30 次；A/B 交替顺序，至少取得 3 对有效轮次。未出现指定按钮、未实际触发过渡或缺少关键记录的轮次单列。
4. 长测每组 10～15 分钟，轮次一致，起止都在同一地图关闭背包。记录最后一个旧补间自然结束与正常 GC 周期完成后的值，比较退出活动和资源保留。仅有周期汇总时，逐帧 p95、p99 留空。
5. 本试点的允许常驻对象包括仍在使用的共享补间引擎、其他控件的动画和当前打开的背包。若要限制 SWF 缓存，需先取得明确的空闲资源所有者、工作集及复用频率。

## 历史原件与回写验收

用 `python src/tools/swf-patches/inventory.py --output build/swf-patches/acceptance/inventory.json` 重建历史清单，与首份输出直接比较一致。31 份历史启动日志包含 18 条活动面板完整响应记录，涉及 4 个资源；另有背包原件和旧修改版各一次本地替换命中。后两条证明当时使用过相应本地文件。本次官方版本见上方实机采集。[取证清单](evidence/swf-lifecycle/inventory-20261121/inventory.json)

用 `python src/tools/swf-patches/roundtrip.py` 从归档原件在两个新目录分别重建。该脚本固定 Temurin 21.0.12.1+1 与 FFDec 24.1.0 nightly 3326，定位到 `PackageSkinManager.dispose` 的 method 224、body 220。整类原样回编使解压内容多出 880 字节，已排除。方法级 P-code 原样回写产生两个逐字节一致的 1,525,029 字节文件，原件为 1,524,948 字节。解压后两者的 2,197,440 字节完全一致，覆盖 SWF 舞台信息、3,357 个顶层 tag 和唯一的 DoABC。这个版本用于测量回写工具的影响。[回写结果](../build/swf-patches/roundtrip-pet-skin/20261121/report.json)

重新从原件导出的 16 个背包类和 3 个 TweenLite 相关类，均与历史导出逐行一致。背包有 6 处位移补间创建点，时长 1 秒，均未设置业务完成回调。实际视图按钮调用 `changeStyle(classify, false)`，这条分支更新标签；初始化等其他路径才会请求背景。自然结束后，TweenLite 每经过 60 次根帧更新清扫已结束的索引项。[方法与所有权记录](evidence/swf-lifecycle/pilot-20261121/ownership.json)

## 首版生命周期候选

创建位移补间的六处代码在返回的补间句柄上记录拥有者。标记复用每个 Motion 自己创建的位置对象。关闭面板时管理器逐个释放 Style，Motion 只停止拥有者标记相同的句柄，同时解除目标引用。清理已关闭实例时，仅当该实例仍是当前单例才清空单例。其余所有者可以继续在同一控件上播放动画。

修正版由 `python src/tools/swf-patches/patch_skin.py` 从归档原件生成。独立重建得到完全相同的 1,525,194 字节。静态校验核对完整 ABC 和其余所有 tag，只允许一个 ABC 内 1,502 个方法体中的 13 个修改，以及相应的局部寄存器数量和公共名称。首版因后向跳转缺少 AVM2 的 `label` 指令被运行器拒绝，修正版的静态检查也覆盖这个条件。[构建复核](../build/swf-patches/pet-skin-lifecycle/reproducibility.json)

以下结果来自本仓库编译的 Ruffle release 测试器，观测夹具以 Sprite 代替背包美术，执行真实归档的 PackageSkinManager、Style、Motion 和 TweenLite 字节码。先运行一次无逻辑修改回写对照，然后按 A/B、B/A、A/B 各启动一个独立进程测量。每轮 5 次预热、30 次提前关闭，7 次运行全部通过。自然结束、快速复开和同一目标上其他拥有者的动画另设断言。[三对短测逐轮记录](evidence/swf-lifecycle/harness-series-20261121T090436586674Z/sequence.json)

| 局部指标 | 原件 | 修正版 |
| --- | ---: | ---: |
| 90 次关闭后仍活动的位移补间 | 90 | 0 |
| 90 次关闭后旧控件继续移动 | 90 | 0 |
| 暂时关闭 TweenLite 的自动覆盖，连续切换产生 3 个位移补间，关闭后仍活动 | 3 | 0 |
| 同目标其他动画逐个恰好完成一次并达到终值（含预热） | 105/105 | 105/105 |
| 五种位移自然结束、再返回原点 | 全部通过 | 全部通过 |
| 旧管理器二次关闭后，新单例仍可用 | 否 | 是 |
| 等待正常清扫后，索引中保留的旧目标 | 0 | 0 |

两组最终都能清扫旧目标。此次改善集中在关闭与原定动画结束之间，不能据此得出长期内存或帧率结论。隔离试验没有调用完整面板的业务通知，没有覆盖实际游戏画面、输入、GPU 及账号所在场景。[主验收独立重建的功能候选运行记录](evidence/swf-lifecycle/harness-20261121T090503159204Z/report.json)

## 空闲资源与缓存

本次实收的背包有 152 份脚本。12 份 AS 的反编译输出有缺失，已用全部 152 份 P-code 和唯一 DoABC 的字符串常量池复核。模块内对 `PackageCacheManager` 的唯一外部直接调用在关闭入口，调用的是 `dispose`；直接写入或查询这个缓存均为 0 次。13 份皮肤控制脚本未直接引用 `BitmapData` 或此缓存。模块中唯一的 `cacheAsBitmap` 赋值用于显示中的属性图标背景；138 个嵌入图像定义也无法给出当前空闲显存。[当前原件的缓存所有权](evidence/swf-lifecycle/live-capture-20261121/cache-ownership.json)

位移层持有 UI 控件、位置和短时补间，继续使用共享索引的自然清扫。暂不增加对象池或缓存预算。若长测发现增长，需在实际写入或借用的一侧量出在用条目、空闲条目、估算字节和重建次数。历史进程的 10 次完整 GC 快照仅提供全局资源盘点。皮肤背景的异步加载另有调用链。[历史资源所有权](evidence/swf-lifecycle/inventory-20261121/ownership.json)

## 首版官方原件采集（已完成）

此次登录使用首版实验包中的 `run-capture.cmd`，取得了两份官方响应。今后官方更新时，可用采集入口登录并打开背包，让宿主保存游戏实际收到的下列资源。

| 所需原件 | 采集包内的位置 | 用途 |
| --- | --- | --- |
| 背包模块 | `runs/capture-*/swf-capture/newpetpackage~<实际版本>.swf` | 确认管理器、13 个目标方法、视图按钮及关闭调用链的版本 |
| 补间库 | `runs/capture-*/swf-capture/library_util~<实际版本>.swf` | 确认 TweenLite 的调度表、拥有者标记、停止和清扫语义 |
| 本次加载结果 | `runs/capture-*/resource-load.json` | 查看每项的实际版本、网络／缓存来源、与历史原件的直接比对结果及拒用原因 |

每次启动使用独立数据目录。仅配置开关、只取得其中一份 SWF、没有本轮网络命中或游戏异常退出，都不足以确认两项当前资源。若内容或版本变化，先用新原件重新定位并生成控制件，再决定是否适用相同补丁。对外复核只需两份 SWF 和结果 JSON。

## 首版实验包与回退

实验包位于 `dist/aola-swf-lifecycle-20261121`，构建标记为 `swf-lifecycle-pilot-v1`。五个入口使用相同 EXE，固定为 50 fps、子树实验开关关闭。功能候选只在手动选择的入口启用。

| 入口 | 用途 | 有效命中条件 |
| --- | --- | --- |
| [run-capture.cmd](../dist/aola-swf-lifecycle-20261121/run-capture.cmd) | 在当前会话采集两份官方响应 | 两个资源均有本轮网络完整响应，含版本、字节数和归档比对结果 |
| [run-official.cmd](../dist/aola-swf-lifecycle-20261121/run-official.cmd) | 回到官方资源 | 退出旧进程后，用此入口重新启动 |
| [run-original.cmd](../dist/aola-swf-lifecycle-20261121/run-original.cmd) | 历史原件 A | 当次背包的精确来源、版本、完整字节与归档一致 |
| [run-roundtrip.cmd](../dist/aola-swf-lifecycle-20261121/run-roundtrip.cmd) | 无逻辑修改回写对照 | 当次背包原件一致，且控制件解压内容与原件完全一致 |
| [run-lifecycle.cmd](../dist/aola-swf-lifecycle-20261121/run-lifecycle.cmd) | 功能候选 B | 同一播放器中先确认 util，再确认背包。两份官方实收内容均与归档一致，候选仅含编入程序的固定改动 |

受限加载器直接比较实收原件和试点参考，校验 42 处已审变更及全部未变化区间。错源、换版、缺失依赖、内容不符或命中缺失会使本轮无效。真实材料通过 14 项离线 navigator 专项回归，覆盖实际 fetch 缓存路径和两个播放器之间的隔离。独立 HTTP 回环用例在本机因 Windows 10013 受阻。真实游戏的 HTTP 命中、回退后显示和交互需由登录后的正常会话验收。[专项通过记录](evidence/swf-lifecycle/loader-regression/frontend-accepted.log) · [回环记录](evidence/swf-lifecycle/loader-regression/loopback-final.log)

交付时逐字节复核包内四份 SWF，EXE 的 `--help` 正常退出。另以 26 个诊断样例核对运行结果判定，覆盖依赖顺序颠倒、缺失命中、错误字节数、版本变化和异常退出。首次上线采集前，正式登录器和子树实验仍沿用各自既有入口。[包内清单](evidence/swf-lifecycle/capture-20261121/package.json) · [诊断样例结果](evidence/swf-lifecycle/capture-20261121/summary-tests.json)
