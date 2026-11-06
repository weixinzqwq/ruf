# 奥拉星原生迁移：登录与进图

本轮将奥拉星官方 Flash 启动链接入现有 Rust/Ruffle 框架，先验证官方登录与进入地图。游戏由原生 Ruffle/DX12 执行和绘制，WebView2 承接官方 HTML 登录。CEF 项目、原版软件和已交付奥奇版本保留。

构建底座是奥奇最新的 `command-batch-test-20261105` 优化源码，路径为 `../aoqi.1.03/reconstructed/engine`。奥拉模式把启动 URL、网页登录桥和游戏页面上下文替换为奥拉的调用链；渲染器继续使用这一份优化引擎。`aola-helper` 仅用于读取原 CEF 实现及网页登录接口的参考代码，不是构建依赖或运行时子进程，也未调用其中的旧 Ruffle EXE。

已逐项核对上一轮交付清单中的 33 个渲染及 wgpu 后端文件，全部与优化基准一致，见 `evidence/optimized-framework-audit.json`。DX12 命令合并按通道用途和附件条件启用，不依赖奥奇宿主开关；纹理池、滤镜绑定复用和资源维护同样位于共用后端。这证明代码继承关系，不等于奥拉实战帧率已经测量通过。

## 启动与构建

当前测试目录为 `dist/aola-window-move-test-20261106`。本轮修正 Windows 拖动、缩放窗口时游戏停止更新的问题，使用同一时钟继续执行播放器更新与绘制。继续提供 `run-detail-A-original.cmd`、`run-detail-B-optimized.cmd` 两个 50 FPS 入口，SWF 和上轮配置保持一致；两者均包含本轮窗口修正。说明见 [窗口测试说明](WINDOW-MOVE-TEST.md)。此前测试版均保留。

默认 960×560、高画质、DX12、1 倍速，帧率由原版 SWF 控制。可选 50/60 FPS 入口会覆盖官方帧率调整，也可能加快按帧计时的动画。没有移植抓包、主动发包、战斗辅助、主题或形态替换。

源码复用 `../aoqi.1.03/reconstructed/engine`，不是另一份未同步的引擎副本。当前源码包含回收、画布复用、帧更新计时及资源库生命周期修正，使用 `tools/build.ps1 -Variant window-move` 构建；脚本拒绝覆盖已保留的测试 EXE。A 版的独立源码差异和快照保存在 `evidence/gc-pool-20261105/gc-progress-source`，仅修改输出目录名不会撤回源码中的画布优化。`tools/build.ps1 -Action test` 执行网页登录桥的相关检查。使用已有离线依赖与 Rust 1.99.1。

独立数据目录为 EXE 旁的 `aola-data`，可以通过 `--aola-data-dir` 指定隔离验收目录。静态游戏资源缓存复用当前框架；不导入旧 CEF 数据。网页登录使用独立的临时会话，Flash SharedObject 使用内存后端。诊断记录阶段、接口名、页面路径、计数、数字错误码和慢回调的编译函数元数据，不记录 Cookie/SID 值、查询值、回调参数、密码或包内容。

## 已核对的官方 SWF 调用链

2026-11-05 从官方公开站点取得的版本根为 `20261029761034522`。运行时仍由原版 start 获取当时的版本 XML，下表是分析快照，不是把版本固定到宿主代码中。

```text
play.html + js/aola.js
  └─ start~2107300001.swf / mmo.checkversion.Load
       └─ start~<时间戳>.xml
            └─ loader~<lv，缺省取 v>.swf / mmo.loader.LoaderContainer
                 ├─ difference~<v>.xml → WebLoginChecker
                 │    └─ showWebLogin → 官方 HTML 登录 → game_auth_sid
                 └─ loader_res~<lv 或 v>.swf
                      └─ GameBootLoader.init
                           └─ versiondata~<v>.swf
                                ├─ config~<v>.xml
                                └─ library/socketserver1~<资源版本>.swf
                           └─ 配置中的基础库 + user/login
                                ├─ 原版登录、选服与游戏 Socket
                                └─ play~<覆盖后的资源版本>.swf
                                     └─ mmo.play.DocPlay.init → BootLoader.init
                                          └─ SocketClient 初始化与角色数据
                                               └─ DocPlay.onBootComplete
                                                    └─ SceneManager → 首张地图
```

基础库与 `play` 有预加载和并行步骤，上图表达依赖关系，不表示每个请求都串行发生。`LoaderContainer.onCompleted` 会预加载 `play`；看到其下载成功不等于账号已进入地图。

| 关键点 | 当前证据 |
|---|---|
| 原网页入口 | `source/play.html` 指定 960×560 的 `start~2107300001.swf`；`source/aola.js` 定义查询、Cookie 和登录回调 |
| 版本选取 | `decompiled/start/scripts/mmo/checkversion/Load.as`：远程加载时间戳 XML，取 `lv`，否则取 `v`，以 `loadBytes` 加入当前 ApplicationDomain |
| 网页登录 | `decompiled/loader/scripts/mmo/loader/util/WebLoginChecker.as`：读取 `game_auth_sid`，缺失时调用 `showWebLogin`，成功后按原流程删除一次性 Cookie |
| 资源索引 | `GameBootLoader.as` 加载 versiondata；`scheduler/VersManager.as` 先读全量表，再由 `start.xml/u/f` 覆盖同名资源 |
| 通信模块封装 | `GameBootLoader.onLoadSocket` 去掉下载内容的前 8 字节，再 `loadBytes`。保存的原件位于 `source/library_socketserver1~2026052196362970.bin`，提取副本位于 `variants/socketserver-inner.swf` |
| 当前主程序 | 全量表的 `play=2026102936286521` 被热更新覆盖为 `261031358096721`；已下载的是覆盖后的文件 |
| 游戏入口 | `LoaderContainer.onLoadedLogin` 实例化 `mmo.user.login.Main`；`play` 的文档类是 `mmo.play.DocPlay` |
| 实际进图 | `play/DocPlay.init` 创建 `BootLoader`；完成后 `onBootComplete` 按账号分支调用 `SceneManager.loadScene/loadDefaultScene`，收到 `SCENE_INITED` 后关闭加载界面 |

当前资源地址及来源见 `metadata/startup-resources.json`；只有标记 `fetched=true` 的资源已经实际取得。原始公开文件在 `source`，头部/压缩检查在 `metadata`，反编译在 `decompiled`。FFDec 对个别与启动路径无关的方法出现反编译失败，未把这些输出重新编译或替换到游戏中。

## 宿主适配

- 复用旧 `aola-helper/analysis/RuffleCompatibility/host/aola.rs` 已实现的页面桥语义，接入当前框架独立模式；不搬入旧实验的渲染补丁和包解析器。
- 保留 `getQS/getU/getCode` 的原始查询值、`null/undefined` 差异，以及 WebUtil 把参数包成单个 Array 的调用方式。
- 在官网自己的登录页，将依赖 Flash 开场动画的 `action=startgame` 选为官网已有的 `action=login`，显示官方 HTML 表单；输入、验证码与账号验证由官网处理。
- `SeqLoader` 在 `ShaderJob` 完成回调中读取正常通信序列所需的结果。本引擎此前同步计算后未派发完成通知；现对默认模式补发 `complete`。这不是实现完整异步 ShaderJob 调度，仍保留现有同步计算方式。接口背景见 [AIR SDK](https://airsdk.dev/docs/development/display/working-with-pixel-bender-shaders/using-a-shader-in-stand-alone-mode)。
- Socket 字节保持原样。官方 `SocketServerClient` 自行消费策略响应，宿主不剥离它，也不接管登录协议。
- 奥拉模式补上 Windows 中文字体回退，奥奇字体路径保持原样。
- 沿用当前框架的临时纹理复用、局部混合画布、绑定共享、稀疏资源表清理和命令列表合并。没有据此宣布奥拉的性能已经通过。

实现文件位于共享框架的 `desktop/src/aola_host.rs`，入口与窗口接点位于 `main.rs`、`app.rs`、`player.rs`、`custom_event.rs`、`cli.rs`。开始修改前的七个接点文件保存在 `source/framework-before`。

## 验收状态

### 奥拉背包详情解析与调用定位版（当前）

精确版本的官方 SWF 小范围补丁与同引擎 A/B 入口，逐响应记录 AVM2 方法、原生调用和同步 GPU 资源工作。在线收益及兼容性等待手动验收；静态与构建证据在 `evidence/pet-detail-20261106`。

### 奥拉完整更新与慢回调定位版（上一轮）

增加播放中 tick 的互斥阶段计时及总量核对，记录帧循环外慢回调的编译函数来源、调用次数、累计和最长耗时，并记录定时器补跑情况。此版用于定位此前约 292 ms 的未细分更新时间，实际在线结果待手动验收。见 [测试说明](TICK-PROFILE-TEST.md)。

### 奥拉切换动画与高帧率对照版（上一轮）

单独启用已停止离台子树跳过，保留默认官方帧率，增加明确的 50/60 FPS 与关闭跳过的对照入口。依据上一版日志，孤立对象工作和绘制都存在明显耗时；本轮实际提速与动画兼容性待在线验收，不能以目标帧率代替实测。见 [测试说明](SWITCH-FPS-TEST.md)。

### 奥拉帧更新与资源库生命周期版（上一轮）

新增帧更新阶段互斥计时和工作计数；电影拥有符号库、类拥有符号映射，全局索引改为弱引用。库盘点和无效索引清理在完整回收结束后进行。自动检查及交付结果见 `evidence/frame-library-20261105/delivery.json`；实际长测改善待用户在线验收，不能以编译或回归检查代替。详见 [测试说明](FRAME-LIBRARY-TEST.md)。

### 奥拉回收与画布复用对照版（上一轮保留基准）

修正回收标记完成后可能被连续根修改阻止进入清扫的问题；增加累计创建、内容销毁和分配释放计数。另保留单项对照版本，通过局部画布尺寸归档降低动画尺寸变化时的纹理重建。测试与实战边界见 [本轮对照说明](GC-POOL-TEST.md)。

用户已完成两版第二战同场景测试，均反馈流畅。实测回收周期持续推进，B 的显示帧率约 29.31 FPS、每次绘制纹理新建较 A 减少约 31%。B 窗口面积较 A 小约 6%，首战和长期稳定性不能据此直接确认。保留 B 为后续测试基准、A 为对照；详见 [实测报告](GC-POOL-RESULTS.md)。

随后十分钟混合战斗/录像长测出现明显卡顿：前后五分钟显示 FPS 为 25.55 / 17.42，帧更新为 10.59 / 25.35 ms。回收继续完成，但 SWF 资源库及部分图形资源持续增长。B 只保留为短测基准，长期稳定性未通过；持有链证据、场景差异和下一轮范围见 [长测报告](LONG-RUN-RESULTS.md)。

### 奥拉模型性能定位版（上一轮保留基准）

从本轮起以奥拉为优化目标。用户采集的 5507 对木桩场景平均约 7.93 FPS；原版没有输出引擎分阶段性能记录，无法只凭 PresentMon 区分帧逻辑、位图准备和 CPU 侧渲染提交。

新增奥拉入口专用的 5 秒性能采样，记录 PID、时间、实际帧率设置、逻辑/呈现帧率、帧更新、绘制、滤镜、命令转换、提交、资源创建/占用和 GC。没有改动画质、游戏速度或渲染策略；这是定位版，不是已经证实提速的优化版。完整分析与采集方式见 [战斗模型性能记录](BATTLE-MODEL-PERFORMANCE.md)。

### 单窗口修正版（已验收基准）

上一版遗漏了与奥奇一致的窗口结构：官方登录 WebView2 与 Ruffle 是两个顶层窗口，因此此前的进图结果只能证明加载链能运行，不能算窗口迁移完整。

本次沿用奥奇的 `build_as_child` 方式，将登录页作为 Ruffle 主窗口内的 WebView2 子控件。移除额外窗口创建和独立窗口事件分发；子控件跟随主窗口尺寸及 DPI 变化，登录成功只隐藏子控件并交还键盘焦点，不隐藏主窗口；关闭主窗口时先释放 WebView2，再释放 Ruffle 窗口。渲染后端及 SWF 调用链保持不变。

本次 Release 构建、3 项奥拉桥接检查及 25 项原宿主回归检查通过。实际启动时仅发现一个属于新程序的顶层窗口；日志记录子控件创建、官方登录页显示、登录完成后子控件隐藏，窗口截图显示游戏已在原主窗口内运行。用户进一步确认“同一窗口，缩放和操作都正常”，因此单窗口登录切换、窗口缩放和游戏点击/移动已通过用户实机验收。关闭流程已完成代码核对，当前登录窗口保留，未强制关闭账号进行测试。

当前 EXE 的 SHA-256 为 `894f75a58eaba52fe91e38532658b83b194ab5dc85894d90403f8868ee20abba`，交付结果见 `evidence/embedded-window-delivery.json`。旧双窗口版本保留在原目录，测试请使用上面的最新目录。没有将这次窗口修正作为帧率或战斗性能验收。

### 首版进图记录（双窗口，历史结果）

2026-11-05：官方入口、当前版本表、加载器、登录模块、主程序和通信模块已完成静态链路核对；Release 构建成功，3 项奥拉网页桥检查和 25 项原奥奇宿主回归检查通过。首版 EXE 的 SHA-256 为 `d6ce9929a79c180f094637333f19383b195ab0e4d936d5f90ecddb7580bf2519`。

本轮实际启动的是本目录构建出的 `dist/aola-native-test-20261105/aola-native-test.exe`，独立验收数据位于 `acceptance/first-entry`。脱敏日志记录了官方登录页、登录完成后页面上下文交回和原生播放器重新加载。用户随后明确确认“已进入地图，可以移动”，因此官方登录至进图、基本鼠标移动已由用户实机验收；不是仅凭编译、下载或加载器界面判定通过。

尚未完成奥拉帧率、战斗、长时间显存稳定性、完整中文输入及所有界面验收。此轮日志中出现一次 Flash `Error #2007`，以及未接入的网页兼容性查询 `isChrome80CookieIssueUser`；当前日志只保留错误码，不能据此定位具体调用，用户反馈的进图和移动未因此阻断。这些留作后续兼容性排查依据。

原 CEF 项目的其他辅助功能尚未迁移，后续逐项处理。
