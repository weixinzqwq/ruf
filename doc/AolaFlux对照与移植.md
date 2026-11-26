# AolaFlux 对照与移植

2026-11-22，适用于 `src/optimization` 性能实验线。合作方提供了[公告专项分析](../AolaFlux_notice_analysis.txt)和[静态分析](../AolaFlux_v1.0.0_static_analysis.txt)。下面的成本和行为数据来自本项目的实测。在线 FPS、点击至新模型首帧及长时间运行后的内存基线仍待同场景采集。

## 运行与对照

下表保留 Flux 历史运行包的设置。后续 MyPro 包继承这些演出开关，并提供普通战斗和 3v3 的额外入场背景转场对照。两个包的性能数据分别归档。[MyPro 运行入口及验收](MyPro对照与移植.md)

11 月 23 日运行包新增战斗变身立绘开关。启动后再切换模式需要重新启动播放器。

| 启动入口 | 目标变身立绘 | 指定公告 | 万域之潮攻击演出 | 后台解压 ZWS |
| --- | --- | --- | --- | --- |
| [常规游戏](../dist/aola-flux-20261123/run-game.cmd) | 跳过 | 显示 | 播放 | 开 |
| [原版立绘对照](../dist/aola-flux-20261123/run-original-portraits.cmd) | 播放 | 显示 | 播放 | 开 |
| [屏蔽指定公告](../dist/aola-flux-20261123/run-hide-notices.cmd) | 跳过 | 屏蔽 | 播放 | 开 |
| [关闭万域攻击演出](../dist/aola-flux-20261123/run-skill-fx-off.cmd) | 跳过 | 显示 | 跳过 | 开 |
| [精简特效](../dist/aola-flux-20261123/run-reduced-fx.cmd) | 跳过 | 屏蔽 | 跳过 | 开 |
| [前台解压对照](../dist/aola-flux-20261123/run-control.cmd) | 跳过 | 显示 | 播放 | 关 |

这些入口使用同一 EXE、DX12 High、50 FPS 目标、官方背包和同一个包内 `aola-data`。首次会建立新数据目录，此后共用该目录下的公开资源缓存。它们都保留上轮 B 组的 goto 叶节点优化。这一组实验不能单独比较该优化、纹理池维护或首帧管线的收益。[构建流程](工程构建与迁移.md#AolaFlux-参考优化实验包)

公告开关按启动进程读取。从屏蔽入口切回常规入口可恢复原有公告初始化。查看 `aola-data/diagnostics/startup-*.jsonl` 中 `type=notice-swf` 的记录。只有 `result=hit` 表示实际返回了公告候选。`ready` 仅表示候选已经准备好。随后 `type=notice-channel` 标识该播放器实际执行的通道，每种通道只记录一次。每次运行另写 `performance-*.jsonl`，本构建的 `fluxPortBuild` 为 `aola-flux-v1`，开关见 `optimizationMode`。

## 公告实际省去了什么

11 月 19 日官方资源清单中 `messagesend/messagesendservice` 的版本为 `2026062579184482`。本次从同版原件重建六处编辑。开启后，仅对官方域名、资源路径和版本匹配的完整 GET 响应比较原始字节。命中后交付候选。版本变化、重定向、分块读取、原件缺失或内容不符，按原响应处理。公共缓存保存官方数据。

| 方法 | 开启屏蔽后的行为 |
| --- | --- |
| `MessageRandomShow.initSystemdMsg` | 在读取随机公告表及构造轮询、显示定时器前返回 |
| `MessageRandomShow.onSystemMsgComes` | 已初始化的轮询定时器继续运行，本次回调跳过文本与显示对象构造 |
| `MessageSendService.onHonorMsgDispatch`、`onTitleHandle`、`onReturnBroadcast` | 在读取事件内容、拼接文案和创建队列项前返回 |
| `MessageSendService.messageComing` | 空队列跳过宿主查询。非空时清除尚未取出的公告，保留正在显示的状态 |

独立重建的候选均为 304,392 字节。离线 AVM2 回归运行六个实际方法，测试副本将宿主调用指向可控布尔值，并开放观察字段。开启时两个随机定时器未创建，非空队列从 2 变为 0。测试宿主返回 false 后，原有称号和回归处理产生了 4 个队列项。该结果覆盖公告业务行为。游戏里的系统消息、聊天及其他模块有各自的显示路径。[补丁脚本](../src/tools/swf-patches/patch_notice.py) · [行为夹具](../src/tools/swf-patches/test_notice_patch.py)

## 加载与渲染的实测

ZWS 读取器把 LZMA 输出缓冲直接交给 SWF 解析器，省去一遍完整输出复制。桌面电影加载路径可通过 `AOLA_BACKGROUND_ZWS=1` 在现有两个资源线程、16 个等待槽的队列上解压。默认引擎关闭此独立选项。它保留公开 HTTP 缓存的原始响应和游戏的 ABC 执行方式。真实背包的标签在两条路径逐字节相同，并确认工作线程由主线程变为资源线程。`performance-*.jsonl` 的 `processResourceWorker` 统计整个进程已完成的资源任务及其累计执行时间。在包内固定的 `sleep` 模式下，可观察 ZWS 后台任务是否实际发生。任务时间不含排队，不能直接换算为少掉的卡顿时间。

| 单个资源，每版本 6 次计时 | 旧版 → 本版解码时间中位数 | 每次少申请的累计堆字节 | 解码器峰值 |
| --- | --- | --- | --- |
| 11 月实收背包 | 118.8 → 113.8 ms | 2,197,569（2.10 MiB） | 均为 6,414,913 字节 |
| 10 月战斗原件 | 396.4 → 429.7 ms | 6,573,674（6.27 MiB） | 均为 14,985,322 字节 |

两件资源在 ABBA 交错运行中的耗时变化方向相反。已测得的收益是减少累计分配，以及启用后台时把解码移出调用线程。旧战斗文件仅作为解压样本。[逐次解码耗时与分配口径](evidence/flux-20261122/zws-abba.json)

原桌面启动先构造 Low 的 1× 舞台管线，再由播放器选 High。本版直接准备所选画质的管线，并预建首帧最终拷贝。实际 DX12 选择了 4×，8×8 首帧的 64 个像素全部符合预期。同一设备上随后显式切 Low 才增加 87 个原生渲染管线。这项收益发生在冷启动。重复切换同一模型仍使用管线缓存。[DX12 测试输出](evidence/flux-20261122/dx12-first-frame.txt)

临时纹理池按最近使用帧安排到期检查。常规帧跳过大批未到期的桶，返回的纹理仍受共享空闲预算约束。项目原有显示位图预算为 384 MiB、临时纹理池预算为 256 MiB。它们统计的资源类型不同，两个数字都不能作为进程总显存上限。[池的策略和历史实测](回收与纹理复用.md)

## 报告各项如何处理

| 报告项 | 当前项目的处理及适用边界 |
| --- | --- |
| E01 / E13 样本和观察工具 | 保留项目现有 DXGI、逻辑帧、渲染和详情回调采样。实验沿用项目已有的画质设置 |
| E02 / E03 解压 | 单缓冲及可选后台 ZWS 已接入。项目随附依赖为 `lzma-rs`。本次没有引入原生 liblzma，也没有把磁盘上的 ZWS 永久改写成 FWS |
| E04 磁盘 | 各入口共用本包公开资源缓存，按总预算后台清理 |
| E05 / E06 / E07 显存 | 复用原有显示位图预算、分配前回收和 DXGI 统计，优化空闲池到期检查。DX12 压力实验只调整可回收缓存 |
| E08 管线 | 启动时按选定画质创建舞台管线，并预备最终输出所需的 1× 复制管线。本机 DX12 High 使用 4× 采样，省去了旧启动顺序预建的 87 条 Low 舞台管线 |
| E09 / E10 战斗表现与清理 | 按当前官方原件 `petfightnew~261112849126071.swf` 定位万域之潮攻击演出，独立开关。当前原版在已确认的战斗面板退出路径上已清理电影与补间管理器 |
| E11 公告 | 同一启动开关控制上述五个通道、六个方法。屏蔽生效范围限公告模块 |
| E12 Socket | `AOLA_SOCKET_BUDGET_MS` 默认关闭，正整数以毫秒计。每次 socket 更新至少处理一个已到达的事件，达到预算后其余按 FIFO 留待下轮。单条慢回调仍会执行完，持续流量超出处理能力时可能增加延迟 |

## 缓存和分帧开关

| 环境变量 | 包内默认 | 生效方式 |
| --- | --- | --- |
| `AOLA_DISK_CACHE_BUDGET_MB` | 4096 MiB | 启动时后台检查，新写入触发的检查在 30 秒窗口内合并。超额后按文件最后写入时间，清理到预算约 80%。只识别本包 `cache/v1` 下的公开资源条目，跳过临时文件、其他格式与链接。0 关闭清理 |
| `AOLA_DXGI_CACHE_PRESSURE` | 关 | 设置为 `1` 时，DX12 至多每 2 秒读取一次当前进程的本地段用量与预算。达到 90% 后临时空闲池上限减半，并尝试收回冷位图。连续 3 次不高于 70% 后恢复配置。压力策略保留滤镜正常分配的上限 |
| `AOLA_SOCKET_BUDGET_MS` | 关 | 设置正整数后分批处理事件。用 2 ms 做对照时，可在包目录执行下面的 PowerShell 命令。三种优化可分别测试 |

```powershell
$env:AOLA_SOCKET_BUDGET_MS = '2'
& .\run-game.cmd
Remove-Item Env:\AOLA_SOCKET_BUDGET_MS
```

## 战斗表现和已存在的回收路径

输入是 `resources/original/petfightnew~261112849126071.swf`，ZWS 原件 5,377,898 字节，完整 FWS 为 6,573,705 字节。`AOLA_BLOCK_SKILL=1` 单独控制 `GQWanyuZhichaoEffectHelper.playAttackMovie` 的 31 字节守卫。`aolaBlockSkill("wanyu-zhichao")` 返回 true 时跳过万域之潮攻击演出，并调用非空完成回调。11 月 22 日仅带这条守卫的候选为 6,573,810 字节。11 月 23 日的运行包把这条守卫和变身立绘编辑放入同一战斗候选，两个开关独立读取。

任一战斗显示开关开启后，第一次收到正确版本、完整且逐字节匹配的响应时，后台从官方原件构造候选。同一播放器的后续加载复用构造结果。两个开关均关时直接使用官服响应。实机可在启动日志中用 `type=battle-swf, result=hit` 确认资源已替换，再用 `type=battle-effect, name=wanyu-zhichao` 确认跳过了万域攻击演出。验收应包含触发该技能后进入下一步和战斗结算。

本版官方 `FightPanel.close`（方法体 4164）和场景切换的 `removeFromStage`（4180）都会调用带防重检查的 `dispose`（4165）。派发战斗或回放结束事件前，`disposeManagers`（4071）依次清理电影、逐帧事件、Tween、帧率四个管理器。电影管理器（241）移除逐帧脚本、停止片段、从父对象移除并清空列表和日志。Tween 管理器（472）暂停并终止补间、重建列表并清除单例。这个退出路径已具备合作方报告中有实质作用的两项清理。强制 GC 或再执行一次清理没有加入。异常退出和实机长测覆盖另计。[长时稳定性验收](资源库与长时稳定性.md)

## 战斗变身的全屏立绘

2026-11-23 对照用户截图和归档的官方 `resources/original/petfightnew~261112849126071.swf`。触发类是 `mmo.petfightnew.common.fightcontroller.ChangePetView`。FFDec 对两个方法的 ActionScript 导出失败，以下资源路径、类名和回调关系从 AVM2 P-code 核实。方法体编号对应这份原件。

| 截图 | 立绘素材请求 | 加载方法 | 播放入口 |
| --- | --- | --- | --- |
| 图一，异刃首次变身 | `gqpet/yiren/effect/yirenfirstappeareffect5225~1210.swf` | `showYiRenMovie`，方法 1814、方法体 1812 | 加载成功回调 1812 将 `mmo.yiren.effect.YiRenFirstAppearMovie5225` 交给 `ActMovieHelper.showAndPlayMcMv` |
| 图二，旧日重现 | `gqpet/jiurichongxian/effect/jiurichongxianeffect_5625~1210.swf` | `showJiuRiShenAppearMovie`，方法 1762、方法体 1760 | 加载成功回调 1760 将 `mmo.gqpet.jiurichongxian.effect.JiuRiChongXianAppearEffect_5625_0` 或 `_5625_1` 加入模态层，再调用 `gotoAndPlay(1)` |

普通变身响应的入口是 `FightController` 的 `RSPN_BATTLE_POWERUP` 分支。`ChangePetView.changeView` 更新亚比数据并加载新模型，`onGotPet` 等模型就绪后进入 `onTalkMoviePlayed` 的类型分派。两条截图路径均先请求共享 `petfightres/changeviewres~2020012349049159.swf`，然后按异刃和旧日重现类型各自加载立绘。截图里的 `pet5226_2_*` 和 `pet5625_2_*` 属于模型位图资源。异刃立绘后缀来自 `GQPetType.getYiRenObject(...).viewId`，所以资源后缀 `5225` 可以和新模型号 `5226` 不同。旧日重现使用 `curPet.raceId % 10000` 拼出素材后缀。

### 跳过立绘时如何完成变身

开关 `AOLA_HIDE_TRANSFORM_PORTRAIT=1` 作用于下列五条加载路径。素材编号仍由游戏计算，因此同一方法处理的其他形态也适用。截图中的两个素材编号是实例。

| 模型替换时的演出 | `ChangePetView` 方法 | 跳过时的行为 |
| --- | --- | --- |
| 异刃首次变身 | `showYiRenMovie`（1814） | 写入本场首次标记，复用闭包 1813，执行 `onPetChange` 换入模型、启动原缩放补间，再调用完成回调。已有标记时仍走原版 `yirenappeareffect` 局部特效 |
| 觉醒 | `showJuexingAppearMovie`（1757） | 复用闭包 1756 换入模型并通知完成 |
| 旧日重现 | `showJiuRiShenAppearMovie`（1762） | 复用闭包 1761 换入模型并通知完成 |
| 登神长阶 | `showDengShenChangJieAppearMovie`（1767） | 复用闭包 1766 换入模型并通知完成 |
| 真理法典 | `showZhenLiFaDianAppearMovie`（1772） | 复用闭包 1771 换入模型并通知完成 |

后四条原本在资源加载成功后，把影片加到模态层再播放。开启开关时，它们在请求资源前调用各自原有的回退路径，先执行 `afterPlayAppear()` 替换战场模型，然后调用 `callBack.invoke()`。完成通知沿 `onEffectMoviePlayed()` 进入 `FightPanel.onGotPet()`。

有些变身还会在换模分派之前播放一段独立影片。对白 `showTalkMovie`、星辉觉醒大电影 `showXinghuiJuexingBigMovie` 和 `showDegeneratorMovie` 在 `MOVIE_LAYER` 播放，结束后才进入换模分派。它们继续按原版播放。表中的觉醒分支对应 `GQJueXing`。奥义、常规现形和异刃重复变身的特效挂在战宠层 `layerPet`，随模型位置播放。

旧模型恰好在战宠层最上方时，原版回退闭包移除它以后，按旧索引加一插入新模型会越界。初版在真实显示列表中复现 `RangeError #2006`。最终补丁在跳过演出且旧模型处于顶层时校正传给闭包的索引，使新模型仍能放在顶层；旧模型位于中间时沿用原版叠放顺序。[负对照](evidence/transform-portrait-20261123/depth-negative-control.txt)

最终战斗候选为 6,574,247 字节。两次独立重建得到相同内容。与 11 月 22 日的万域之潮候选逐方法比较，5,620 个方法体中只有上述五个发生变化。真实 `ChangePetView` 方法在隔离 AVM2 中通过 32 个场景，覆盖两组动态素材、开关、异刃重复变身及中间、顶层、唯一子对象三种显示层次。启用时目标立绘请求为零，模型换入后完成回调恰好执行一次。恢复立绘时记录到了原资源请求，测试用受控资源回退完成换模。真实美术播放、技能按钮和下一回合仍需在线核对。[方法差异](evidence/transform-portrait-20261123/method-comparison.txt) · [回归结果](evidence/transform-portrait-20261123/portrait-regression.txt) · [分类场景](evidence/transform-portrait-20261123/portrait-cases.txt) · [万域之潮回归](evidence/transform-portrait-20261123/wanyu-regression.txt)

实机对照时，用常规入口触发相同变身，观察新形态、技能按钮和下一回合。要恢复全屏立绘，使用同一目录的 `run-original-portraits.cmd`。两次启动共用资源缓存。`startup-*.jsonl` 的 `battle-swf / hit` 表示战斗候选已交付，`battle-effect` 下的 `transform-portrait-*` 名称标识本次实际跳过的分支，每种只记录首次。版本更新或同版本官方内容变化时，播放器继续使用收到的原件。

发布包内已核对五处带顶层索引修正的守卫和两份原件。六个启动入口分别用默认和自定义磁盘预算运行，12 次参数检查通过。实际资源加载入口交付的战斗候选与独立重建的文件相同；桌面宿主的 11 项回归覆盖了立绘和万域之潮开关的组合。[发行包核对](evidence/transform-portrait-20261123/packaged-executable.json) · [启动入口](evidence/transform-portrait-20261123/launch-flags.json) · [资源加载](evidence/transform-portrait-20261123/resource-fetch.txt) · [宿主回归](evidence/transform-portrait-20261123/desktop-host.txt)

## 在线验收

固定地图、亚比顺序、窗口和画质。先用常规入口测试首次载入，再重复同一组已缓存详情。用对照入口重复，交替先后顺序，并记录每次 `petDetailResponses` 的阶段与耗时。要观察公告，则只更换公告入口，核对实际命中和通道记录。上一轮记录到了单次超过 1 秒的详情回调，公告和分帧开关的结果应与该回调单独分析。[已有实机耗时](背包详情与SWF去重.md#11-月-22-日复核)

## 11 月 22 日初版验收记录

| 验收层次 | 观察到的结果 |
| --- | --- |
| 项目标准回归 | `build.ps1 -Component optimization -Action test` 通过。子树脚本休眠关闭和开启时，核心用例各通过 36 项；桌面宿主与诊断通过 10 项，包含公告开关和通道日志。[用例结果](evidence/flux-20261122/project-tests.txt) |
| 原件重建和 AVM2 业务 | 公告候选两次独立重建得到相同字节。万域之潮候选两次也相同。真实公告类在可控宿主下运行六个方法。万域之潮在有回调和空回调两种输入下，开启时业务服务访问为 0，完成回调共执行 1 次。关闭后两次调用都沿原版无特效目标分支正常返回。[公告回归](evidence/flux-20261122/aola_notice.txt) · [技能回归](evidence/flux-20261122/aola_wanyu.txt) |
| 资源加载接入 | 离线使用真实归档，从播放器 navigator.fetch 入口验证关、开、再关。缓存仍为原件。当前版候选与 FFDec 独立产出完全相同，换版本或改变同版本响应字节走原件。[公告](evidence/flux-20261122/fetch-notice.txt) · [战斗](evidence/flux-20261122/fetch-battle.txt) |
| ZWS 读取和调度 | SWF 库 96 项通过。独立开关只选择 ZWS，背包和历史战斗原件分别对照旧版输出。递归加载同一 ZWS 的 AVM2 测试在前台、后台两种模式均通过。两组各有 3 个已有 loadBytes 用例通过。[读取器回归](evidence/flux-20261122/swf-reader-tests.txt) · [前台](evidence/flux-20261122/recursive-zws-0.txt) · [后台](evidence/flux-20261122/recursive-zws-1.txt) |
| 纹理池与 DX12 | 渲染库常规 36 项和 4 项真实 DX12 池测试通过。压力实验读取了真实 DXGI 用量，在同一渲染器注入高低读数，验证冷缓存回收、借出句柄有效和预算恢复。没有制造整机显存耗尽。[池测试](evidence/flux-20261122/dx12-pool.txt) · [压力开关测试](evidence/flux-20261122/dxgi-enabled.txt) |
| Socket 分帧 | 4 项预算与 FIFO 测试覆盖慢事件、关闭顺序和回调期间的新到事件。16 个已有 AVM1/AVM2 Socket 用例在预算关闭和 1 ms 两组各通过 16 项。[分帧回归](evidence/flux-20261122/socket-dispatch.txt) · [1 ms 实例](evidence/flux-20261122/sockets-1ms.txt) |
| 磁盘清理 | 在临时目录超过额度后，5 个条目中最旧的 2 个被释放，覆盖冲突发布、临时文件和未知文件保留。Windows 实物符号链接与 junction 夹具受环境权限限制。链接跳过逻辑只有代码核对。[临时目录验收](evidence/flux-20261122/disk-budget.txt) |
| Windows 包 | 打包后的 EXE 和两份资源均与验收输入逐字节相同。5 个 CMD 入口经 Windows 命令解释器运行，核对了选项、含空格路径及退出码传递。发行包用独立数据目录启动，完成 WebView2 创建和页面加载，止于登录前。[启动参数](evidence/flux-20261122/launch-flags.json) · [宿主启动](evidence/flux-20261122/desktop-smoke.json) |
