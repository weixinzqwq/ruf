# Aola 项目进度与新对话交接

更新时间：2026-11-07（北京时间，用户指定优先恢复gotoAndStop内部定位；goto-profile独立测试版已构建，待用户采集）。本文件供新对话接续任务；优先阅读本节及第12节，无需重做此前的迁移与窗口修正。

## 1. 当前准确状态

- **当前轮次按用户指定优先定位 `gotoAndStop` 内舞台/orphan构建、脚本遍历、广播与清理。** 新增逐详情响应的阶段计时和访问计数，保持原执行顺序，不宣称已经改善帧率。卡片统计遍历留后续；角色面板日期修正保留，但尚无在线验收反馈。
- 当前优化对象是 **Aola（奥拉星）**，不再以奥奇为优化与验收对象。底层仍复用已经优化过的 Rust/Ruffle 源码。
- 已完成官方登录、单窗口内嵌 WebView2 与游戏显示。用户此前确认可以进地图、移动，窗口缩放及操作正常。
- **最新窗口拖动修正已由用户确认没有问题。** 用户原话：“当前窗口修正后没有问题，但是我还没有进行刚才替换swf以及使用官方swf的AB组测试。”
- **本轮 A/B 已完成并分析，实际替换命中已确认。** 每次详情响应少2次读取/解析，但帧率未明显改善；原始间隔重算 A/B 呈现约12.04/12.33 FPS，显示变化约11.92/11.94 FPS。用户反馈操作条件相同、均无显示异常；实际窗口像素尺寸不同，且 B 在采集后有错误码，不标记完整在线兼容性通过。
- **面板故障已定位：游戏的 `2024/1/5 02:00:00` 被引擎拒绝，起始赛季日期无效，空赛季列表触发初始化 #1009。** 用户用诊断版在00:26左右连续三次复现，目标资源均成功取得，前两次完整错误链吻合；第三次堆栈受日志上限截断。当前用 `date-compat` 版修复四位年格式中一位月/日解析，在线显示仍待用户验收。
- 当前源码保留既有渲染/回收优化、完整 tick 计时、逐次详情响应定位、可选择的 SWF A/B 替换、窗口拖动修正与实际 SWF 命中诊断、面板定向诊断和上一轮日期兼容修正；本轮新增goto内部阶段计时。新入口明确关闭本地背包替换和面板逐方法日志，未修改官方活动SWF。
- 根目录及旧窗口版的四份 A/B 脚本已统一设置 `AOQI_STAGE3_MODE=sleep`，新 `ab-verify` 包沿用修正后的入口。本轮发现旧 Aola 宿主会过滤实际命中消息，此次已使用新 EXE 完成 A/B，并在 startup 日志确认 `pet-swf-replacement`，不是仅凭 `petPatchMode` 判断。

上一轮先备份并修正四份脚本，随后核对发现旧命中日志不可达，故仅补充专用诊断并以新目录交付；SWF、游戏逻辑、渲染、回收和窗口修正均未再修改。脚本准备证据在 `evidence/ab-ready-20261106`，其最初“ready”结论已标记被后续日志核对取代；最终源码备份、差异、测试与构建记录在 `evidence/ab-logging-20261106`。旧 EXE、SWF、用户数据和既有采集保留，未启动游戏登录；在线测试由用户完成。当前面板诊断轮次见第10节，不能将这段历史说明理解为当前未修改源码。

## 2. 项目位置与边界

| 用途 | 位置 |
| --- | --- |
| Aola 交付、证据和说明根目录 | `D:\game\Rea\aola` |
| 实际 Rust/Ruffle 引擎源码 | `D:\game\Rea\aoqi.1.03\reconstructed\engine` |
| 引擎构建脚本 | `D:\game\Rea\aoqi.1.03\reconstructed\tools\build_engine.ps1` |
| Aola 打包脚本 | `D:\game\Rea\aola\tools\build.ps1` |
| 用户 PresentMon 采集目录 | `D:\game\Rea\aola\capture` |
| 用户提供的历史 SWF 优化参考 | `D:\game\Rea\aola\swf` |
| 历史 CEF 项目，仅作参考 | `D:\game\aola-helper` |
| 奥奇原版，应保留 | `D:\game\Rea\aoqi.1.03\Air奥奇传说1.0.3.exe` |

用户明确要求从优化后的奥奇 Ruffle 框架迁移 Aola，不能调用 `aola-helper` 的旧 CEF 程序来冒充迁移。当前游戏由原生 Ruffle/DX12 执行，WebView2 承接官方网页登录。框架没有把所有在线游戏 SWF 固定编译进 EXE；游戏仍走官方资源版本与加载链，下面的背包替换是明确选择的单项本地资源覆盖。

用户手动完成官方登录和在线游戏测试。不要自动操作已有登录窗口、替用户战斗或复制登录凭据。保留旧 EXE、原始 SWF、采集记录与用户数据。后续修改保持单项、可对照，不混入无关功能或重构。

## 3. 当前应使用的版本

**当前背包卡顿定位使用 `goto-profile` 版。**

- 入口：`D:\game\Rea\aola\dist\aola-goto-profile-test-20261107\run-goto-profile.cmd`。
- build：`aola-goto-profile-test-20261107`；schema：`6`。构建、哈希和验证见 `evidence/goto-profile-20261107/delivery.json`。
- EXE：38,739,456字节；SHA256：`1ffdff9065b0cf0527f9d890985addce5009c2c5c5e7e52793bbf90cf1acacc1`。
- 独立数据目录 `aola-data-goto-profile`，sleep、50FPS，启用 `AOLA_GOTO_PROFILE=1`，关闭背包替换和上一轮面板逐方法日志。
- 固定窗口尺寸，静置5秒，背包按固定顺序查看约8只亚比、每次停约2秒，同序再做一轮；关闭背包等5秒后正常退出。无需SWF A/B或额外PresentMon。完整操作见 `GOTO-PROFILE-TEST.txt`。

### 保留的角色面板日期修正版

- 入口：`D:\game\Rea\aola\dist\aola-date-compat-test-20261107\run-date-compat.cmd`。
- build：`aola-date-compat-test-20261107`；schema：`5`。EXE大小38,687,744字节；SHA256：`dfaa20339411b6b4da84acc37066287c1ddbd56d04db4491d9dc68e28bd0975b`。完整核对结果见 `evidence/panel-date-fix-20261107/delivery.json`。
- 使用独立 `aola-data-date-compat`，保留窗口修正、sleep、50 FPS及定向诊断，显式关闭本地背包替换。
- 手动登录，打开角色信息面板，若显示则检查页签及年份/赛季切换，正常关闭后告知结果和时间。无需发包或PresentMon；操作说明 `DATE-COMPAT-TEST.txt`。

### 已完成复现的保留诊断版

- 入口：`D:\game\Rea\aola\dist\aola-panel-diagnostic-test-20261107\run-panel-diagnostic.cmd`。
- EXE大小38,687,744字节；SHA256：`1e51c46396841c37d612bb37c884b6235365f539e2c30d43cd9ee1c3ac26fcee`。完整验收记录见 `evidence/panel-personalinfo-20261106/diagnostic-build-20261107/delivery.json`。
- build：`aola-panel-diagnostic-test-20261107`；schema：`5`。
- 使用独立 `aola-data-panel`，保留窗口修正、sleep、50 FPS，显式清空 `AOLA_PET_PATCH`。这是定位版，不用于比较此前 A/B 性能。
- 手动登录，按原操作打开一次角色面板，等待约5秒后正常关闭，告知复现时间。无需发送协议命令或采集 PresentMon。

### 已完成 A/B 的保留版本

此前使用保留窗口修正的 `ab-verify` 版完成 A/B。旧 `window-move` 版缺少实际命中记录，不用于正式对照。

- 目录：`D:\game\Rea\aola\dist\aola-ab-verify-test-20261106`
- EXE：`D:\game\Rea\aola\dist\aola-ab-verify-test-20261106\aola-native-test.exe`
- 文件大小：38,670,848 字节。
- SHA256：`b5a3cf85716ff77a0e44b82e9b2cf4b1b672a197bf680030fd07b7ac85b364da`，本轮交付已核对。
- 日志 build：`aola-ab-verify-test-20261106`；schema：`5`。

| 入口 | 实际含义 | 独立数据目录 |
| --- | --- | --- |
| `D:\game\Rea\aola\dist\aola-ab-verify-test-20261106\run-detail-A-original.cmd` | 同一命中诊断修正版 + 保存的官方背包 SWF 原件 | EXE 旁 `aola-data-baseline` |
| `D:\game\Rea\aola\dist\aola-ab-verify-test-20261106\run-detail-B-optimized.cmd` | 同一命中诊断修正版 + 仅两处重复解析删除补丁 | EXE 旁 `aola-data-optimized` |

两者均含窗口修正、强制 `--frame-rate 50`；纹理池空闲预算 256 MiB，近期帧数 120。Aola 宿主固定 DX12、高画质，初始尺寸 960×560。A 也是通过本地精确替换返回官方原件，避免 A/B 分别走网络与本地造成混淆。

上一窗口版 `D:\game\Rea\aola\dist\aola-window-move-test-20261106` 已保留，EXE SHA256 为 `a5f9ec40d57c20ea2c480e2e226df2be049436cb9f9ccf7bb04a80913cd55cc7`。旧版 `D:\game\Rea\aola\dist\aola-pet-detail-test-20261106` 已保留，EXE SHA256 为 `c038da64a88577fbe0dc0b116c05f05847c6174d038df9a87a7d0d729522a65c`。更早的 tick 定位版、其他对照版也保留。不要将新旧引擎混成 SWF A/B。

### 测试入口已修正（本轮交接）

上一轮交接记录的两个错误变量名为：

```bat
set "AOQI_STAGE3_baseline=sleep"
set "AOQI_STAGE3_optimized=sleep"
```

本轮已将 Aola 根目录与旧窗口版目录内的四份脚本统一修正为 `set "AOQI_STAGE3_MODE=sleep"`，每份脚本仅改变这一行。其他参数、A/B 模式和独立数据目录均保留；新 `ab-verify` 包复制根目录修正后的两份入口。根目录脚本供打包复制使用，根目录当前没有 `aola-native-test.exe`；用户应使用上表中的新 `ab-verify` 版完整路径启动。

此前错误拼写不代表 sleep 必然关闭：`desktop/src/main.rs` 在 Aola 初始化时调用 `initialize_aola_mode()`，`core/common/src/aoqi_work.rs` 的 Aola 默认值就是 `sleep`，但外部继承的 `AOQI_STAGE3_MODE` 会优先。修正后两个入口均显式覆盖该变量；仅变量名修正本身无需重编译 EXE，本轮新 EXE 是为补充实际命中诊断。测试后仍需在两份运行日志中确认 `optimizationMode` 都为 `sleep=true, loading=false, dedup=false`。

两份 SWF 与旧窗口版一致，旧 EXE 和旧详情测试包保持原样。最终交付核对记录见 `evidence/ab-logging-20261106/delivery.json`；最初脚本改前哈希及已有采集/日志清单见 `evidence/ab-ready-20261106/before.json`。本轮未联网重新核对官方版本，实际替换是否命中仍以用户此次运行日志为准。

用户测试步骤见 `D:\game\Rea\aola\dist\aola-ab-verify-test-20261106\START-A-B-TEST.txt`。该说明针对当前 `ab-verify` 版；目录内沿用的 `PET-DETAIL-TEST.txt` 中旧交付目录名称只代表上一轮历史，不能据此退回旧 EXE。

不要把 `AOLA_PET_PATCH=baseline` 误认为“所有引擎优化关闭”；它只选择官方背包原件。`run-aola-50fps.cmd` 本身不设置 SWF 替换，正常干净环境下用于在线原资源路径核对；从已有命令行启动时仍应检查是否继承了 `AOLA_PET_PATCH`。

### 本轮补充：实际替换命中记录

源代码核对发现，Aola 在 `desktop/src/main.rs` 中安装专用 `DiagnosticLayer` 后提前进入应用循环，不会执行普通 `cache/log` 文件日志初始化；原过滤器只接受 WARN/ERROR 和 avm_trace，故原 INFO 级别的 `Aola pet SWF replacement hit` 与窗口 enter/exit 消息不能按原交接说明读取。已有数据目录没有普通运行日志，与这一控制流一致。

本轮只为实际返回本地 SWF 字节的事件增加固定 `aola_pet_patch` target，并由宿主过滤器放行。`aola_host.rs` 只提取布尔 `optimized`，写入既有 `diagnostics/startup-*.jsonl`，事件为 `type="pet-swf-replacement"`，另外保留 `elapsedMs`；不记录任意消息、资源 URL 参数、账号或登录值。A 应出现 `optimized=false`，B 应出现 `optimized=true`。初始化选择模式不产生该事件，只有实际替换返回字节才产生。

窗口移动本身已由用户确认，本轮不改窗口逻辑，也不额外启用窗口 INFO 或全部运行日志。新增命中过滤/字段限制回归 1 项、既有 Aola 宿主检查 5 项、窗口嵌套消息循环回归 1 项、release 构建及 `--help` 启动检查已通过；本次在线命中已由用户测试日志确认，性能结果见第8节。

## 4. 上一轮卡顿定位的结论

用户最初在 50 FPS 下静态画面可以达到目标，但背包查看/切换亚比非常卡，战斗换亚比动画也曾下降。优化优先级转为拆解整个 tick 与详情响应，不能只归因于显卡性能、JIT 缺失或 GC。

完整 tick 定位的历史证据位于：

- `D:\game\Rea\aola\evidence\tick-profile-results-20261106\analysis-output.txt`
- `D:\game\Rea\aola\evidence\tick-profile-results-20261106\summary.json`
- `D:\game\Rea\aola\evidence\tick-profile-results-20261106\timeline.csv`
- `D:\game\Rea\aola\evidence\tick-profile-results-20261106\analyze.py`

对应采集：`D:\game\Rea\aola\capture\pmcap-aola-native-test.exe-261106-174050.csv`，关联 PID 33564。该历史采集 Present FPS 约 15.63、显示变化 FPS 约 13.64，存在 5 次超过 1 秒的呈现间隔。先前提到的约 292 ms 是定位前的耗时疑点，不是已经证明的某个定时器耗时。

前次完整 tick 分析中，匹配区间的 sockets 阶段占主要耗时，重点指向 `PetDataService/onViewSinglePet` 及其嵌套详情派发。不能把父子回调的 inclusive 时间直接相加，也不能仅凭 33 次详情监听回调认定重复注册。

绘制仍是第二条优化线：此前匹配段约 22.4 ms/次；另一份强制 50 FPS 日志存在约 219–382 ms/次的绘制区间。这些属于既有分析，**不是本轮 A/B 结果**。以后应按同一场景和时间段重新核对，不把单项详情解析收益扩大为所有绘制问题已解决。

此前交付准备时 `capture` 最新是上述 17:40:50 采集。窗口版数据目录已存在运行日志；本轮已在 `evidence/ab-ready-20261106/before.json` 记录已有日志及采集文件名、大小和修改时间，以便后续识别新测试。这些既有记录不是本次 A/B；本次应使用第8节列出的23:25及23:27采集。

## 5. 当前官方 SWF 核对与实际补丁

### 版本与原件

2026-11-06 上轮实施时从官方 `start~<时间戳>.xml` 与 versiondata 核对：

- 版本根：`20261105915135290`。
- `pet/newpetpackage`：`2026102936286521`。
- `pet/petdataservice`：`2026091780747880`。
- 这些是当时的版本快照，不保证未来仍是官方当前版本。

交付包内：

| 文件 | SHA256 |
| --- | --- |
| `swf/newpetpackage-original.swf` | `5011d9a09e6b5cd1e82b1bae36ffc6ad0ce84a1f89db3b998a61b308cbfdbd87` |
| `swf/newpetpackage-detail.swf` | `308cf278d40ce80001cbbf2e9d2e4bc35ecbf120cc379c17fe4767cd926f122c` |

### 已核实的嫌疑点与取舍

- 当前官方 `PetDetailDataMap.getPetDetailInfo()` 仍然创建新的 `PetDetailedInfo` 并调用 `parseInfo()`，不是仅根据 9 月反编译代码猜测。
- 当前 `PetDataService/onViewSinglePet` 也完成创建、解析、缓存更新及事件派发。
- 用户给的 `D:\game\Rea\aola\swf\01-current\Patches\newpetpackage-morph-20260919895706432-visual-v3.fws.swf` 是较早版本，其 `PetDetailDataMap` 与当前核对结果一致，没有解决此处重复解析。
- 历史参考的 `SimpleViewController` 有销毁标记、异步回调版本保护和显示对象清理，但混合了形态桥逻辑，不能整体覆盖新版。本轮没有移植这部分。
- **没有做全局共享详情对象缓存。** 部分面板会保留、修改详情对象，例如 HKTipsPanel 修改 `_pbi.isLockHk`；贸然共享会改变对象别名语义。

### 本轮只改了两处

在当前官方 `PetCardList.onPetDetailedCached` 和 `GodCardList.onPetDetailedCached` 中，各删除 `checkCache(...)` 分支内紧随首次读取的一次重复 `getPetDetailInfo(_loc2_.id)`。保留每个消费者自己的详情对象、事件顺序、其他方法及美术资源。

补丁通过 P-code 重建。最终正确候选是：

`D:\game\Rea\aola\evidence\pet-detail-20261106\variants\newpetpackage-detail-v4.swf`

不要拿该目录中最初候选、v2、v3 继续交付：它们分别有未导入 P-code、方法参数元数据丢失、提取错误 GodCardList 方法的问题，均已拒用并留作证据。v4 的两个目标是 GodCardList methodIndex 414/bodyIndex 406，PetCardList methodIndex 638/bodyIndex 630。

静态验证：152 个脚本中只有目标两行行为变化；3357 个 SWF tag 中只有索引 3353 的 DoABC 改变；全部非代码 tag、资源、SymbolClass 和时间轴契约保持一致。参数签名核对通过，重复构建得到一致哈希。这些检查不等于在线兼容性与性能收益已通过。

重建脚本：`D:\game\Rea\aola\evidence\pet-detail-20261106\rebuild-swf.ps1`。

### 替换生效条件

代码：`frontend-utils/src/backends/navigator/aola_pet_patch.rs`，由 `desktop/src/player.rs` 的 Aola 分支接入。

仅接受 `aola.100bt.com` 的 HTTP/HTTPS 默认端口、无用户名密码、GET 且无请求体、路径恰好为 `/play/pet/newpetpackage~2026102936286521.swf` 的请求。本地文件 SHA256 必须一致，响应保留原 URL。其他版本、域名、方法仍走原网络路径。

**新 `ab-verify` 版必须在 `diagnostics/startup-*.jsonl` 找到 `type="pet-swf-replacement"`，核对 A 的 `optimized=false` 和 B 的 `optimized=true`。** `petPatchMode` 只表明选择了哪种模式，不是实际加载命中的证明。官方若更新文件名，本补丁不会覆盖新版；没有命中时先核对官方版本，不要盲目放宽到任意版本。

## 6. 新增详情计时怎么解读

主要源码（相对上述引擎根目录）：

- `core/src/pet_detail_profile.rs`：逐次详情响应、方法调用计数、自身/包含子调用时间、资源计数差值。
- `core/src/tick_profile.rs`、`core/src/player.rs`：tick 阶段与范围衔接。
- `core/src/avm2/function.rs`、`core/src/avm2/method.rs`：方法边界计时及编译函数名缓存。
- `desktop/src/aola_diagnostics.rs`：schema 5 日志输出。

响应根入口为 `PetDataService/onViewSinglePet(` 和 `PetDetailDataMap/tryDispatcherCurrentDetailedCachedEvent(`。缓存直接派发也会被记录；嵌套根调用归入外层响应，避免当成两个独立响应。直接由输入触发的缓存切换标为 `outsideTick`，不会错误并入 tick 阶段总计。

性能日志中的 `petDetailResponses.responses` 每项包含：

- `sequence`、`stage`、`totalMs`。
- `methods` 中的 `source`、`category`、`calls`、`inclusiveMs`、`selfMs`、`maxMs`。
- `resourceCreationMs`、`timedResourceCreations`、`meshRegistrationMs`、`queueSubmitMs`。
- `bookkeepingMs`、`unlistedCalls`、`unlistedSelfMs`，采样外层还需检查 `droppedResponses`。

分析限制：

1. 方法次数包含快调用，不再只统计超过 5 ms 的回调。按每个响应比较次数，才能区分反复刷新与单次执行很慢。
2. `selfMs` 扣除被记录的子 AVM2 exec 边界；`inclusiveMs` 含子调用，不能逐项相加。
3. `category` 按编译函数名分成 detailData、timeline、text、displayList、listRefresh、resource 等。它不是精准的底层 CPU 分类；TextField 原生方法内部的排版/字形工作仍合在该方法里。
4. 同步资源初始化与提交计数可能和方法时间重叠；网格注册单列，避免把网格持有记录与底层缓冲创建重复相加。响应结束后才完成的异步加载仍看既有 tick/慢回调记录。
5. 每次响应最多保留 1024 种来源，每次采样最多 16 个响应；先检查遗漏计数。
6. 详细计时有额外成本，`bookkeepingMs` 不包含所有计时/函数名提取/日志成本。必须同一 EXE 做 A/B，不能拿 B 与旧诊断版直接宣称收益。
7. 仅记录编译函数元数据与数字计数，不保存回调参数、账号信息或网络包，不新增游戏 GC 对象持有关系。

既有完整 tick、慢回调、绘制、提交、纹理创建/淘汰和显存计数仍保留。

## 7. 已完成的窗口修正

原因为播放器更新只在 `AboutToWait` 执行，Windows 原生拖动/缩放嵌套消息循环期间此入口不派发。

修正位于 `desktop/src/window_move.rs`、`desktop/src/app.rs`、`desktop/src/custom_event.rs`，并在 main 与 Windows 依赖中接入。进入拖动/缩放时启用 10 ms 唤醒定时器，经 winit 用户事件调用同一更新和绘制路径；仍用同一 `Instant` 计算经过时间，不改变游戏帧率。只保留一个待处理唤醒，退出、销毁或释放守卫时清理定时器及窗口钩子。

已通过：原生隐藏窗口嵌套循环回归 1 项、Aola 宿主回归 5 项、release 构建、`--help` 启动检查、旧版与 SWF/启动脚本一致性审计。测试验证普通 AboutToWait 暂停时仍有更新、退出停止、第二次拖动恢复以及守卫释放后停止。

用户现在已确认“窗口修正后没有问题”。`evidence/window-move-20261106/delivery.json` 中的 `manual_online_drag_resize_validation=pending user test` 是交付当时的旧状态；本文件记录的是更新后的用户确认。不要擅自把旧证据改成当时已验收。

证据：`D:\game\Rea\aola\evidence\window-move-20261106`，包含 before、source-after、source.diff、metadata/tests-final.log、metadata/build.log、delivery.json。早期测试停住是隐藏测试窗口退出等待的问题，最终检查已修正并通过。

## 8. 本轮 A/B 结果与下一步

用户已完成测试，并明确反馈“操作条件相同，均无显示异常”。

- A：`capture/pmcap-aola-native-test-A.exe-261106-232516.csv`，PID 7436。
- B：`capture/pmcap-aola-native-test-B.exe-261106-232711.csv`，PID 37920。
- 同一 build/schema、50 FPS、sleep 模式已核对；startup 日志实际命中分别为 optimized=false/true。
- 前7次响应的 getPetDetailInfo 次数317→303，parseInfo次数331→317，每次正好少2次；gotoAndStop均为1689次。
- 原始间隔重算呈现FPS约12.04→12.33，显示变化FPS约11.92→11.94；大于1秒的呈现间隔5→8次。去重生效，但整体卡顿未明显改善。
- 前7次完整详情响应均值约732.73→884.61ms；gotoAndStop自身占比约43.54%→48.35%，全部parseInfo含子调用约占2%。不能把B较慢直接归因于补丁。
- 用户认为操作条件相同，但实际窗口尺寸A=1129×767、B=1217×763，B像素面积约大7.23%，呈现模式分布也不同。以后正式对照需统一实际像素尺寸。
- 每次响应的1024种方法来源上限均触及，未丢失完整响应；未列出的方法不可当作零调用。
- B在采集结束后记录2007一次和1009八次，无堆栈，原因未定，不能直接认定为补丁错误；本次可见显示正常不等于全面兼容验收完成。

用户现已指定继续拆分 `gotoAndStop -> goto_frame_now -> run_inner_goto_frame` 内部的全舞台/orphan构建、脚本遍历、广播和清理耗时，并关联反复切换时的访问规模。槽位增长不是内存泄漏证明；同帧goto有可观察副作用，不得直接全部跳过。卡片数量查询的高频遍历保留为后续方向；绘制仍是第二条优化线。**本轮先计时定位，未跳过或重排时间轴执行。**

A/B分析阶段只分析记录、静态核对调用链和更新进度，没有修改源码、EXE、SWF或启动脚本，也没有启动游戏；后续面板诊断阶段另有源码修改，见第10节。详细报告、输入快照、汇总、逐响应表及图在 `evidence/ab-results-20261106`，入口 `report.txt`。原始采集和交付时的delivery证据保留，未回填历史验收状态。

## 9. 构建、验证和补充资料

- 当前打包命令：`D:\game\Rea\aola\tools\build.ps1 -Variant goto-profile`。
- 脚本默认 variant 也是 goto-profile，且核对源码 build 标记、拒绝覆盖已有交付 EXE；已有launcher分支保留。**交付目录存在时，直接重跑会拒绝覆盖；下一轮需新 variant/标记/目录，不要删除旧包来绕过。**
- Rust 工具链：`D:\game\Rea\aoqi.1.03\toolchain\rust-1.99.1`。
- Cargo 缓存/产物：`D:\game\Rea\aoqi.1.03\reconstructed\build\engine-cargo-home`、`engine-target`；离线依赖配置 `engine\.cargo\vendor.toml`。
- 复用已有脚本中的环境：单构建任务、`RUST_MIN_STACK=8388608`、正确 Java 路径、显式 `CARGO_PROFILE_DEV_DEBUG=0` 和 `CARGO_PROFILE_TEST_DEBUG=0`。此前构建脚本恢复环境时出现过空字符串 profile 配置导致后续测试未启动的问题；相关记录保留，勿误报为引擎测试失败。
- 沿用的窗口回归脚本：`D:\game\Rea\aola\evidence\window-move-20261106\validate.ps1`；文件/哈希审计：同目录 `delivery_check.py`。
- SWF 轮次最终验证共 21 项：core 15、替换路由 1、宿主 5。明细与拒用候选记录：`D:\game\Rea\aola\evidence\pet-detail-20261106\delivery.json`。
- 上轮实际替换命中修正的最终证据：`evidence/ab-logging-20261106`；A/B操作步骤：`START-A-B-TEST.txt`。
- 上轮面板诊断的源码备份、差异、测试、构建与哈希：`evidence/panel-personalinfo-20261106/diagnostic-build-20261107`；操作步骤：`PANEL-DIAGNOSTIC-TEST.txt`。
- 当前日期修复的复现快照、分析、源码备份、差异、修复前后测试与构建哈希：`evidence/panel-date-fix-20261107`；操作步骤：`DATE-COMPAT-TEST.txt`。
- 说明入口：`D:\game\Rea\aola\README.md`、`PET-DETAIL-TEST.txt`、`WINDOW-MOVE-TEST.md`、`TICK-PROFILE-TEST.md`、`BATTLE-MODEL-PERFORMANCE.md`。
- 更早的 GC/纹理池、帧更新与资源持有分析分别位于 `evidence\gc-pool-20261105`、`frame-library-20261105`、`backpack-50fps-20261106` 等目录。需要时再定向读取，不要把历史各版的测试结果混作当前版本结果。

## 10. 角色信息面板故障与下一次复现

用户提供 `71_29`、`vUId`、代码资源 `activityres/activity231229/petfightspace3/petfightspace3_code` 与代码类 `mmo.activity231229.petfightspace3.panel.personalinfo.PFSpace3_PersonalInfoPl`，现象为转圈后不显示；进一步确认 A、B 两组都出现。早先“均无显示异常”是 A/B 模型/属性切换检查反馈，不应据此否定这项新报告。

依据保留的2026-11-06版本映射，独立获取公开代码与界面资源，均HTTP 200、ZWS解压完整、脚本及SymbolClass导出成功：

- `petfightspace3_code~2026110521924920.swf`，64577字节，SHA256 `4e840127663f766ee5bb71311ab16003d3fe792f721378f038ca383ffcbab4a4`。
- `petfightspace3_personalinfo~2026010146590202.swf`，766544字节，SHA256 `6a849643cf956174af2c0928d774e91a1ed6e190cc66d37b2157d83103bce939`。

调用链：代码类 `showPanel -> PFSpace3_PersonalInfo.getPanelInfo -> PFSpace3_UserClient.sendCmdGetUserFightInfo -> CommonClientHandler.sendMsg("71_29")`，数据处理回调后才进入父类显示流程。`PFSpace3_ResConfig` 另映射到 `petfightspace3_personalinfo`，界面类为 `mmo.activity231229.petfightspace3.panel.view.PFSpace3_PersonalInfoPl`，SymbolClass编号371。代码类与界面类同名但包名不同，不得混淆。

`SimpleCustomLoader` 收到完成、失败、取消均可关闭加载提示，只有完成才继续显示回调；`PanelBase.onGetRes` 后续还需 addChild、组件初始化和上舞台。故转圈消失无法区分下载、类创建或初始化问题。公开文件可下载也不证明原游戏会话的请求成功；旧2007/1009缺少堆栈，仍不能绑定具体面板或背包补丁。

此次源码仅增加可开关的定向诊断：`core/src/aola_panel_diagnostics.rs`、AVM2方法边界/异常调用栈、navigator的资源状态与字节数，以及Aola宿主的固定字段日志通道。记录在 `aola-data-panel/diagnostics/startup-*.jsonl`，事件 `type="panel-diagnostic"`。对应活动开始后记录相关方法进入/返回，以及之后发生的AVM2方法错误；错误可能被脚本捕获，须结合后续流程判断。最多8192条加一条上限事件；不输出调用参数、71_29响应正文、登录值或任意异常消息。诊断有额外开销，不用于性能对照。

**本节为诊断版交付时的历史状态；用户现已复现，定位与修复见第11节。** 官方活动SWF未修改或强制本地替换；原A/B包、SWF、采集及用户数据保留。仅获取两份公开静态资源，没有发送游戏命令、启动游戏登录或复用凭据。静态分析细节和证据索引：`evidence/panel-personalinfo-20261106/findings.txt`；不回填旧delivery中的历史pending状态。

已通过：窗口消息循环回归1项、Aola宿主及日志字段检查7项、资源/方法诊断范围检查2项，共10项；release构建与EXE `--help` 检查通过。旧A/B EXE、两份SWF及8份采集/日志文件哈希保持一致，9份改前源码/文档备份校验通过。这些是本地检查，不能代替用户在线复现。

## 11. 00:26面板复现与日期兼容修复

用户反馈北京时间00:26:26左右连续三次点击均失败。对应 `startup-1793982326538-44028.jsonl`，三次代码资源请求约00:26:21.809、23.374、24.951。两份SWF三次均HTTP200且收到完整预期字节数，三次均通过数据解析并进入 `PanelBase.onGetRes`。前两次在 `PFSpace3_DropDownInitHelper.initSubList` 抛错，经 `PFSpace3_UsePetListTab.initPetListTabController -> PFSpace3_PersonalInfoPl.initTabList -> PanelBase.onGetRes` 传播后记录1009。第三次在00:26:25.630达到8192条上限，之后仍记录1009，但没有保留第三份同一方法的堆栈。PID31192稍早会话也有两次完整相同错误链。

具体根因：`PFSpace3_SeasonConfig.FIRST_SEASON_START_TIME` 使用 `new Date("2024/1/5 02:00:00")`，当前引擎的 `core/src/avm2/globals/date.rs::parse_date` 原要求四位年格式的月日都恰好两位，导致无效Date；随后 `fullYear/month` 为NaN，`int(NaN)` 为0，赛季列表为空。`PFSpace3_DropDownInitHelper` 未执行条目创建循环，却读取仍为null的 `_loc9_.height`，中断整个面板初始化。日志中前两个下拉框及同一条目类均已创建成功；赛季列表路径没有季节名/条目构造调用，与此链吻合。头像边框的类查找错误有捕获回退，不是阻断点。

行为修复只将四位年分支的两处月日长度条件放宽为1或2位，保持其它格式、日期归一化及所有游戏资源原样。Adobe avmplus的Date解析实现也接受不补零的数字月日；这里只覆盖本次所需格式，不宣称完整日期兼容。保存原始SWF，不通过更改活动日期常量或跳过空列表来掩盖引擎差异。

修复前，新增3项回归中的日期分词与真实Date构造测试均失败（None/无效日期），旧Flash日期样例仍通过，见 `tests-before.log`。修复后3项日期回归全部通过：核对四种补零组合、三项官方活动日期、空字段/非法字段边界，真实AVM2构造器及固定2026-11-07下赛季编号35/默认年份12个赛季，并执行保留的date_parse SWF与Flash输出对照。加上7项Aola宿主与1项窗口回归，共11项通过，见 `tests-after.log`。最终构建及保留文件核对见 `delivery.json`。

分析与证据入口：`evidence/panel-date-fix-20261107/report.txt`（日志时间及行号）、`root-cause.txt`（源码因果链及范围）、`summary.json`、`timeline.csv`。4份新日志原件快照、改前源码备份、测试失败证据、修复后差异和构建日志均保留。新包使用独立数据目录且保留定向诊断；不自动登录或复制凭据。

**状态：11项回归、release构建、EXE `--help` 及文件核对已通过，角色信息面板在线显示仍待用户验收反馈。** 旧诊断/A-B EXE、两份背包SWF、两份活动SWF、4份新日志与8份旧A/B输入哈希未变，4份改前源码/文档备份校验通过。若该面板恢复，再单独记录其它仍打不开的面板，不能把一项日期修正扩大为全部活动已通过。用户已指定下一轮转回goto定位，日期修正随新包保留。

## 12. gotoAndStop舞台/orphan内部定位

关键缺口：既有 `FrameWork::measure` 仅在 `Player::run_frame` 内激活，而逐详情响应位于sockets或直接输入，不能靠旧frame统计解释这些goto的内部成本。本轮在 `exec_detail_root` 建立响应范围，在 `goto_frame_now` 打开实际goto范围，复用各阶段插点，并补上手动帧脚本执行、已移除对象脚本及数字访问计数。

新模块 `core/src/goto_profile.rs` 使用每响应一个固定大小Box，随 `ResponseCost.goto` 导出，不保留GC对象或逐goto记录。原16个响应/采样上限和droppedResponses仍适用；新统计独立于已触及1024种来源上限的方法表。schema 6的 `petDetailResponses.responses[].goto` 包含实际goto次数、同帧/stop/AVM2次数、排队次数、inner/legacy跳过次数、最大嵌套深度、阶段排他时间及计数。

时间口径：`outerTotalMs` 为本响应内最外层实际goto总墙钟时间；各 `phases[].exclusiveMs` 通过暂停/恢复父阶段避免嵌套重复，求和应等于它，`unaccountedMs`应为0。`inclusiveMs`含重复的嵌套时段，不能再相加。旧AVM方法selfMs扣除所有被测子调用，新阶段中可包含构造器/监听脚本，两者口径不同。记录本身有开销，不把新旧EXE帧率差异视作优化收益。

对象口径：MovieClip构建/脚本访问次数不是唯一对象或新建对象数；orphanVisits为弱引用槽位扫描，executed/invalid/sleepSkipped另列。orphan指没有父节点，隐藏或屏外但有父节点的对象仍在普通树内。orphanSlots仅在goto边界取样，增长不是泄漏证明。广播派发次数不是监听函数数，清理前槽位与移除量分别记录。

保留原有顺序和特殊分支：同帧goto仍执行可观察副作用；inner goto脚本顺序仍为舞台先、orphan后；SWF<=9提前返回及排队行为保持。`run_goto`的其它直接入口不会自行打开统计范围；发生在已打开范围内的嵌套工作才归入当前阶段。排队后在响应结束外执行的goto不记为本响应的实际耗时。

源码/证据目录：`evidence/goto-profile-20261107`。`before.json`及before保存13份改前文件；`design.txt`记录调用链、范围、字段和后续分析方式。核心25项、桌面8项、窗口1项合计34项检查通过，覆盖确定性时钟扣重、禁用及重置、6个Flash样例在开启/关闭统计时的输出、frame外同帧goto，以及既有Aola/日期/窗口检查。窗口项直接运行已通过8项桌面检查的同一测试程序，前后哈希一致，见window-test.log/json；其余见tests-final.log。最终构建、源码差异及新包哈希见delivery.json，不以构建锁等待日志冒充通过。

此轮交付基于 `aoqi.1.03/reconstructed/engine` 源码，不覆盖另一路 `aola/engine` 登录器工作；两路目前共用编译缓存，构建锁等待不等于回归失败。测试导入路径错误的首次日志也保留，后续以tests-final.log为准。

**下一步：用户用goto-profile独立入口完成固定顺序背包切换两轮。先检查每响应goto.enabled、droppedResponses及时间求和，再比较各阶段时间、访问次数和orphan槽位随轮次变化，选择一项有证据的优化。卡片统计遍历暂不实施。** 不自动登录、发包或操作用户游戏，不把离线样例通过扩大为在线性能或兼容验收通过。

## 13. Rust 登录器首期（独立源码）

用户已审核通过基础工具、抓包/发包与最小发送桥方案，并授权初始化本地 Git 保存源码。本期源码在 `aola/engine` 与 `aola/native-core`，从日期兼容修正后的优化版独立复制；第12节的 goto-profile 仍在原工程推进，不能把两条构建线的测试或性能结论混用。

实现中文工具栏、收发记录筛选/详情/暂停、发送文本导入导出、`#send`/`#time` 检查和非阻塞队列，以及帧率/画质/静音与可选日志。Rust 观察原 Ruffle 网络通道；独立启动桥调用游戏自身 `ActClientSender`，沿用原连接。断线、重新登录或观察丢失会停止队列。登录凭据隐藏，不保存原始登录包；日志默认关闭。战斗、主题及形态功能不属于本期。

入口与功能边界见 `LAUNCHER.md`。最终目录为 `dist/aola-launcher-20261107-release`，可双击 `run-launcher.cmd` 或 `aola-launcher.exe`；数据目录独立，原测试包、用户数据与旧工程保留。构建入口 `tools/build-launcher.ps1`，有同名交付 EXE 时要求选择新输出目录。验证证据保存于本机 `evidence/launcher-20261107`。

离线预览已经观察到中文工具栏及收发记录窗口；最终构建与测试结果以 `LAUNCHER.md` 和本轮 `delivery.json` 为准。真实登录、实际收发包和服务器结果仍需用户手动验收，离线模拟不构成在线成功证明。Git 仅在本地保存，未设置或推送远端；历史采集、登录资料和交付程序不纳入版本控制。

本轮最终 37 项相关测试全部通过，Release 构建及 EXE `--help` 检查通过；测试日志为 `tests-release-final.log`。已补齐直接双击 EXE 进入登录器、保存的帧率设置恢复及显式启动帧率优先级。早期编译错误的日志保留，未作为最终通过证据；预览版 EXE 哈希保持不变。
