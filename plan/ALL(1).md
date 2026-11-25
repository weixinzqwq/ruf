# Aola Loader 功能与 SWF 改动

本文以当前工作区的 Aola Loader 桌面入口为准。宿主使用 C#、.NET Framework 4.8（x64）、WinForms、CefSharp 和 PPAPI Flash。功能依据源码、资源清单和本地 SWF 的 FFDec 回读整理。 [宿主项目](analysis/AolaLoader/AolaLoader.csproj)

## C# 宿主与本地 SWF

`Program.Main` 创建 `AolaCompositionRoot`，由它组装浏览器、设置、抓包事件和各功能窗口。SWF 请求由 `AolaResourceRequestHandler` 交给 `ResourceRulePipeline`，按规则返回本地文件或替换后的资源。C# 通过 `AolaPageBridge` 调用补丁注册的 Flash `ExternalInterface` 方法。对战助手和抓包窗口另有一条数据路径，从 `AolaSocketHack` 捕获、重组和解析的消息经 `AolaHookHostAdapter` 传入。 [启动组装](analysis/AolaLoader/AppShell/AolaCompositionRoot.cs) · [资源规则](analysis/AolaLoader/ResourceInterception/Cef/AolaResourceRequestHandler.cs) · [页面桥](analysis/AolaLoader/BrowserHost/AolaPageBridge.cs) · [抓包事件](analysis/AolaLoader/PacketHost/AolaHookHostAdapter.cs)

| 下文简称 | 本地 SWF | 命中的官方资源 |
| --- | --- | --- |
| `petfightnew` | [petfightnew-battle-visual-260929024751831.fws.swf](analysis/AolaLoader/Resources/Patches/petfightnew-battle-visual-260929024751831.fws.swf) | `pet/petfightnew~<任意数字版本>.swf`。当前[清单](analysis/AolaLoader/Resources/Patches/petfightnew-battle-state.current.json)指向视觉 v7，桥接口版本为 5。 |
| `newpetpackage` | [newpetpackage-morph-20260919895706432-visual-v3.fws.swf](analysis/AolaLoader/Resources/Patches/newpetpackage-morph-20260919895706432-visual-v3.fws.swf) | 启用形态桥时匹配 `pet/newpetpackage~<任意数字版本>.swf`。 |
| `start` | [start-bridge-2107300001.fws.swf](analysis/AolaLoader/Resources/Patches/start-bridge-2107300001.fws.swf) | `start~2107300001.swf`。 |
| `controlv2` | [controlv2-theme-live-2026082731219998.fws.swf](analysis/AolaLoader/Resources/Patches/controlv2-theme-live-2026082731219998.fws.swf) | 启用热切换时匹配 `control/controlv2~2026082731219998.swf`。 |
| `play` | [play-panel-capture-2026101541804528.fws.swf](analysis/AolaLoader/Resources/Patches/play-panel-capture-2026101541804528.fws.swf) | `play~<任意数字版本>.swf`。 |

## 战斗数据、动画和背景

| 功能 | 具体效果和 C# 入口 | SWF 文件与函数 |
| --- | --- | --- |
| 对战助手整理敌方数据 | 「扩展 → 对战助手」或 `F3`。C# 的 `AolaBattlePacketParser.TryParse` 解析接收包，`AolaBattleStateTracker.ApplyUpdate` 按宠物唯一编号（UQI）和槽位合并阵容与增量。窗口按「属性」「配置」显示敌方六维、HP、技能 PP/CD，以及装备、魂卡、魂器和晶钥。支持单人 PVP/PVE，配置图片读取本地素材。 [模块说明](doc/modules/BATTLE_SPEC.md#敌方双视图) · [状态合并](analysis/AolaLoader/Features/Battle/State/AolaBattleStateTracker.cs) · [窗口](analysis/AolaLoader/UI/Battle/AolaBattleAssistantForm.cs) | — |
| 隐藏伤害、治疗浮字 | 「扩展 → 战斗表现」分别保存两项设置，`BattlePresentationCoordinator.SaveAndApplyAsync` 经 `BattleBridgeClient.ApplyAsync` 提交。显示开关作用于浮字，战斗结算和「完整信息」仍按原流程处理。 [设置窗口](analysis/AolaLoader/UI/Battle/AolaBattleSettingsForm.cs) · [补丁目标](analysis/tools/petfight/New-PetFightPresentationPatches.ps1) | `petfightnew` 的 `DocPetFight.onAolaBattleApplyOptions` 接收开关。`SkillResultMovieHandler.showIncMovie` 控制治疗浮字，`showReallyHit`、`showShieldHit`、`showReallyShieldHit`、`playStrongMovie`、`setNormalMovie`、`playAllHitMovie` 控制对应伤害浮字，`showMultipleHit` 分别判断两项设置。 |
| 跳过战斗演出 | 「播放战斗过场（出场、替换及技能额外动画）」取消勾选时启用跳过，初始设置即为跳过。客户端直接走部分动画的完成路径，按原顺序结算已收到的事件并更新「完整信息」。 [默认设置](analysis/AolaLoader/Features/Battle/Settings/BattleSettings.cs) · [演出补丁](analysis/tools/petfight/New-PetFightPresentationPatches.ps1) · [出场补丁](analysis/tools/petfight/Apply-BattleVisualPatch.ps1) | `petfightnew` 的 `DocPetFight.onAolaBattleApplyOptions` 写入 `skipBattlePresentation`。各场景的函数见下表。 |
| 精简战斗动画和特效素材 | `BattleVisualResourceRule.Evaluate` 默认替换三份战斗内素材，播放剩余过场时也使用精简素材。进战前的 VS 界面加载官方 `pet/petfightnew_startmv`。 [C# 规则](analysis/AolaLoader/Features/Battle/ResourceInterception/BattleVisualResourceRule.cs) · [迁移记录](analysis/reverse-engineering/battle-visual-migration-20261024/README.md) | `petfightnew` 的 `GetBackgroundUtil.getBackground` 将 `GQcommonBack`、`XinghuicommonBack` 两个背景选择改为 `CommonBack`。 `StartMovie.enterFight`、`StartMovie3v3.enterFight` 清理开场对象后直接调用进战回调，省去特殊背景过场。三个叶子 SWF 的素材改动见下文。 |
| 自定义战斗背景 | 「战斗表现 → 选择 JPEG」。`BattleBackgroundImageResourceRule.Evaluate` 返回居中裁切为 960×560 的本地 JPEG。 [图片规则](analysis/AolaLoader/Features/Battle/ResourceInterception/BattleBackgroundImageResourceRule.cs) · [背景补丁](analysis/tools/petfight/Apply-BattleVisualPatch.ps1) | `petfightnew` 的 `BackGroundViewHandler.setSceneBackGround` 在场景背景上添加 JPEG 加载层，`setBS` 保持站台可见，`changeSkillBackGround` 提前返回以停用该技能背景切换路径。 |

### 跳过演出对应的 ActionScript 函数

以下函数均在 `petfightnew` 中。常规播放由同一个设置控制，并使用当前精简后的素材。

| 场景 | ActionScript 函数与改动 |
| --- | --- |
| 出场与额外特效 | `MCPetContainer.onResPrepared` 直接走出场完成路径，`onStartCriEff` 跳过额外特效。 |
| 换宠与切换形态 | `SwitchPm.showSwitchMc`、`ChangePetView.onGotPet`、`ChangePetView.onPetChange` 走省略过场的换宠或换模型路径。 |
| 击倒与回合切换 | `BattlePmDying.showPetDieEffect`、`BattleNextTurn.checkPlayMovie` 直接进入各自的后续处理。 |
| 技能演出与结算 | `UseSkill.handleUseSkill`、`handleAnnalFeeEight`、`handleMovie` 在相应分支略过奥义、年费或技能演出，继续处理结算和界面更新。 `getMuiltHitDecHpDelayFrame` 将多段扣血的结束延迟缩到一帧。 |
| 队列推进 | `FightController.startHandle` 直接调用 `handle`，省去相邻队列项之间固定的下一帧等待。 |

叶子 SWF 的精简主要发生在素材和时间轴，调用方仍可按原导出类加载。 [素材构建脚本](analysis/tools/petfight/build_my_visual_v7.py)

| 官方资源 | 本地文件与改动 |
| --- | --- |
| `petfightbackground/common` | [common.fws.swf](analysis/AolaLoader/Resources/Battle/Visual/common.fws.swf) 仅保留通用背景导出类 `mmo.petfight.CommonBack`，配合主 SWF 的背景选择。 |
| `common/battleeffmv` | [battleeffmv.fws.swf](analysis/AolaLoader/Resources/Battle/Visual/battleeffmv.fws.swf) 使用透明绘图，保留 `mmo.common.battleeffmv.BattleEffChangeMv` 的 29 帧时间轴和 `mcword` 实例。 |
| `gqpet/shenbing/shenbingmovie` | [shenbingmovie.fws.swf](analysis/AolaLoader/Resources/Battle/Visual/shenbingmovie.fws.swf) 使用 `mmo.gqpet.shenbing.view.LevelUpMovie` 单帧空绘图壳。 |

## 模型替换与背包形态

**战斗模型替换（己方和敌方）**由「战斗表现」按源、目标 `viewId` 保存规则。C# 的 `BattlePetMovieResourceRule.Evaluate` 接管 `fightassets/bitmap/petN/petN_<朝向>_<分片>~<版本>.swf`。每个目标预备正、背两个朝向各三个分片，`BattlePetMovieResourceTransformer.Transform` 将目标 SWF 的动作导出类名和首分片的配置类名适配成原加载方期待的名字。这项改动在素材字节层。战斗中保存的新规则等本场资源清理完成后切换，供下一场使用。 [拦截规则](analysis/AolaLoader/Features/Battle/ResourceInterception/BattlePetMovieResourceRule.cs) · [素材适配](analysis/AolaLoader/Features/Battle/ResourceInterception/BattlePetMovieResourceTransformer.cs) · [规则切换](analysis/AolaLoader/Features/Battle/Coordination/BattlePresentationCoordinator.cs)

**背包形态变化**从「扩展 → 功能面板 → 形态变化」设置。C# 的 `PetMorphCoordinator` 保存规则并向页面推送皮肤、背景及头像映射。模型和海报由 `PetMovieResourceRule` 和 `PetPosterResourceRule` 适配官方叶子素材，隐藏模型使用透明位图。游戏内背包的刷新和皮肤语义由 `newpetpackage` 处理。 [模块说明](doc/modules/PET_MORPH_SPEC.md) · [页面推送](analysis/AolaLoader/AppShell/Adapters/PetMorph/AolaPetMorphLiveBridge.cs) · [模型规则](analysis/AolaLoader/Features/PetMorph/ResourceInterception/PetMovieResourceRule.cs)

| 背包功能 | `newpetpackage` 中的 ActionScript 文件与主要改动 |
| --- | --- |
| 热刷新、背景和皮肤选择 | [SimpleViewController.as](analysis/AolaLoader/Resources/Patches/Sources/newpetpackage-20260919895706432-visual-v3/mmo/petpackage/panel/SimpleViewController.as) 的 `ensureAolaMorphBridge`、`onAolaMorphRefresh` 注册并响应页面刷新，`getAolaMorphVisualSkinIdByRace` 按显示 ID、种族 ID、特殊皮肤 ID 解析目标，`changeBg`、`showPreMovie` 使用更新后的背景和形态过场。 |
| 大图与皮肤交互 | [PetBigIconDetail.as](analysis/AolaLoader/Resources/Patches/Sources/newpetpackage-20260919895706432-visual-v3/mmo/petpackage/panel/inclusion/PetBigIconDetail.as) 的 `getSkinId`、`updatePetIcon`、`onPetIconLoaded` 按目标形态更新视图。 [PackageControlBar.as](analysis/AolaLoader/Resources/Patches/Sources/newpetpackage-20260919895706432-visual-v3/mmo/petpackage/panel/inclusion/PackageControlBar.as) 的 `get skinId`、`getAolaMorphVisualSkinIdByRace`、`updatePetInfo` 让控制栏随目标皮肤更新。 |
| 列表头像 | [PetSingleIconDetail.as](analysis/AolaLoader/Resources/Patches/Sources/newpetpackage-20260919895706432-visual-v3/mmo/petpackage/panel/inclusion/petlist/PetSingleIconDetail.as) 的 `loadYabiFightHead` 经 `getAolaMorphIconId` 取得替换头像，刷新时重新加载。 |
| 面板关闭和快速切换 | `SimpleViewController.dispose`、`clearCommonLiZiMv`、`clearBg` 停止并移除旧粒子或背景。 `refreshUI`、`changeBg`、`showPreMovie` 的延迟回调按视图版本判断是否仍应更新。 |

## 发包、面板、主题与播放

| 功能 | C# 和用户可见行为 | SWF 文件与函数 |
| --- | --- | --- |
| 文本发包与战斗回放 | 抓包窗口可把选中的发送记录加入编辑器，也可粘贴或导入 `#send`。 `AolaTextSequenceRunner.RunAsync` 按组执行并处理 `#wait`、`#time`。 `AolaExecutionState` 保存回放所需的连接和战斗身份，`BattleBridgeClient.GetStateAsync` 读取游戏是否可操作。 [文本执行](doc/modules/ACTIVE_SEND_SPEC.md#统一文本序列2026-10-03) · [C# 发送器](analysis/AolaLoader/Features/ActiveSend/AolaFlashCommandSender.cs) | `start` 的 `mmo.checkversion.Load.ensureAolaBridge` 注册入口，`onAolaSendCommand` 调用当前 Flash 会话的 `ActClientSender.sendMessage`。 `petfightnew` 的 `PetFightService.init` 注册战斗桥，`FightPanel.enableClick` 标记可操作，`DocPetFight.onAolaBattleGetState` 返回战斗状态。 |
| 原生自动战斗 | 文本入口调用 `BattleBridgeClient.SetAutoBattleAsync` 切换原游戏自动战斗，出招和替补交给游戏已有逻辑。 [模块说明](doc/modules/ACTIVE_SEND_SPEC.md#原游戏自动战斗) · [桥补丁](analysis/tools/Add-PetFightNativeAutoBridge.ps1) | `petfightnew` 的 `DocPetFight.onAolaBattleSetAuto` 调用 `setNativeAutoBattle`。原按钮 `NewSkillPanel.changeBtnState` 也走同一个设置方法。 |
| 打开映射面板 | 「抓包 / 发包 → 打开映射面板」把资源路径和 CLS 交给 Flash 加载。 [面板调用](doc/modules/ACTIVE_SEND_SPEC.md#本地面板调用) | `start` 的 `Load.onAolaOpenPanel` 调用原游戏 `Panel_Helper.loadResAndShow`。 |
| 面板调用采集 | 「扩展 → 面板调用采集」显示时间、资源路径和 CLS，可暂停、清空、复制。 `PanelCaptureClient.PollAsync` 轮询页面队列。 [模块说明](doc/modules/PANEL_CAPTURE_SPEC.md) · [修改脚本](analysis/tools/Prepare-PanelCallCapture.py) | `play` 的 `mmo.play.activity.helper.Panel_Helper.loadResAndShow` 在原 p-code 前增加 `aolaPanelCaptured(url, cls)` 回调，随后继续执行原加载指令。 |
| 游戏主题热切换 | 「扩展 → 游戏主题界面」默认提供官方及本地主题 1～10。 `ThemeCoordinator.ApplyAndSave` 保存选择，`ThemeLeafResourceRule` 提供独立的主题叶子 SWF。主题在控制栏就绪时应用，战斗中等待退出。 [主题说明](doc/modules/THEME_SPEC.md) · [主题规则](analysis/AolaLoader/Features/Theme/ResourceInterception/ThemeLeafResourceRule.cs) | `start` 的 `Load.onAolaThemeApply` 转给 `controlv2` 的 `ControlBarSkinHandler.applyTheme`。 `applyLocalTheme` 加载本地皮肤，`activateLocalTheme` 换显示对象，`getCurrentSkin` 读取当前皮肤。 `ControlBarSkinBase.addToStage`、`initializeListeners`、`onGetUrlCompleted` 等接入按皮肤管理的监听，`ControlBarLockHelper` 构造函数支持锁定助手复用。关闭热切换时，主题 1～9 走刷新后的叶子类名适配。 |
| 主题 10「遐蝶·紫夜蝶影」 | 使用独立的 [skin10.swf](analysis/AolaLoader/Resources/Themes/Live/skin10.swf)，显示主题装饰、蝶影、按钮和悬停效果。 [ActionScript 源码](analysis/AolaLoader/Resources/Themes/Sources/Castorice/Skin10.as) | `Skin10` 构造函数添加装饰和显示层。 `onThemeAdded`、`onThemeRemoved` 管理鼠标命中和监听的应用与恢复，`drawEffects` 绘制装饰动效，`styleNotify` 更新通知图样式。 |
| 速度与画质 | 主窗口「速度」提供 25／50／60 FPS，「画质」提供高、最佳。 `AolaMainForm` 通过页面桥应用到 Flash 舞台。 [窗口入口](analysis/AolaLoader/AppShell/AolaMainForm.cs) · [启动桥源码](analysis/AolaLoader/Resources/Patches/Sources/start-2107300001/mmo/checkversion/Load.as) | `start` 的 `Load.onAolaSetPlayback` 设置 `stage.frameRate`、`stage.quality`。 |

## 战斗资源释放与运行诊断

| 功能 | 具体实现与 SWF 改动 |
| --- | --- |
| 战斗内存、资源和帧间隔记录 | C# 的 `BattlePerformanceRecorder` 在战斗开始、结束及观察期间采集 Flash 统计和资源事件。 `petfightnew` 的 `DocPetFight.onAolaBattleGetDiagnostics`、`onAolaBattleDrainTraceEvents` 提供快照和事件。 `FightPanel.dispose`、`FightData.initDomain/dispose`、`StartMovie` 构造函数和 `dispose` 记录面板、战斗域和开场对象。 `PreLoadAssetsUtil.loadAssets/loadNext/onLoadEnd/onLoadCompleted/dispose`、`SkillEffectLoader.getEffect/applyGot/dispose`、`MCPetContainer.attack/setNormal/dispose` 记录预加载、特效和攻击的生命周期。 [采样器](analysis/AolaLoader/Features/Battle/Diagnostics/BattlePerformanceRecorder.cs) · [桥生成脚本](analysis/tools/Update-PetFightBattleBridge.ps1) |
| 战后释放 | 当前 `petfightnew` 视觉 v7 包含 v6 的释放改动。 `PreLoadAssetsUtil.preloadRes`、`onLoadEnd` 和 `LoadFileHelper.preloadSingleFile` 为战斗专用资源登记 Loader，`DocPetFight.processBattleDomainLoaders` 在对应战斗域进入释放阶段后处理它们，`FightMcPool.dispose` 清空池的 `_domain`。历史两场样本仍观测到旧域存活。 [Loader 补丁](analysis/tools/petfight/build_domain_loader_release_v5.py) · [池释放补丁](analysis/tools/petfight/build_domain_pool_release_v6.py) · [历史实测](analysis/reverse-engineering/battle-visual-migration-20261024/README.md) |
| 通用 Flash 统计和内存整理 | `FlashRuntimeClient` 默认读取统计，实际内存整理通过独立设置启用。 `start` 的 `Load.onAolaGetRuntimeStats` 返回内存、帧率、画质，`onAolaTrimMemory` 提交 Flash 的 GC 提示。 [页面桥说明](doc/modules/BROWSER_HOST_SPEC.md) · [C# 客户端](analysis/AolaLoader/Features/Runtime/FlashRuntimeClient.cs) |

另有显式启用的资源归属探针和位图释放实验。前者使用候选 `petfightnew` 的 `DocPetFight.onAolaBattleProbeRelease`，后者替换指定版本 `library/util`，在 `BitmapMovieClip.dispose` 清掉 `_domain` 和 `_frameScriptList`。提示解绑、缓存帧及显示树等 v8～v15 候选的改动和测试记录见迁移文档。 [探针生成脚本](analysis/tools/petfight/Build-BattleOwnerProbe.ps1) · [位图实验规则](analysis/AolaLoader/Features/Battle/ResourceInterception/BattleBitmapReleaseResourceRule.cs) · [各候选记录](analysis/reverse-engineering/battle-visual-migration-20261024/README.md)

## 其他 C# 功能和独立工具

| 功能 | 实现与 SWF 改动 |
| --- | --- |
| 抓包、导出、收发日志 | `AolaSocketHack` 捕获并解析消息，抓包页筛出可导出的发送消息。点击「保留日志」后，`AolaReceiveLogger.TryRetain` 开始落盘收发 JSONL。实现在 C#。 [抓包模块](doc/modules/NETWORK_CAPTURE_SPEC.md) · [日志](analysis/AolaLoader/PacketHost/Consumers/AolaReceiveLogger.cs) |
| 阵容方案 | `LineupCoordinator.ReadAsync` 读取背包实例、顺序、魂卡、魂器和已绑定晶钥，窗口提供 JSON 保存和打开。 `RestoreAsync` 分项恢复并回读，空魂器槽跳过。使用现有 `start` 发送桥。 [阵容模块](doc/modules/LINEUP_SPEC.md) · [协调器](analysis/AolaLoader/Features/Lineup/LineupCoordinator.cs) |
| 宿主窗口和启动资源 | `AolaMainForm` 管理子窗口、窗口缩放、CEF 视口和「保留日志」等入口。 `StartXmlResourceRule.Transform` 在本地响应中改写 `start.xml` 的状态字段。这两项实现在 C#。 [主窗口](analysis/AolaLoader/AppShell/AolaMainForm.cs) · [启动配置规则](analysis/AolaLoader/ResourceInterception/Rules/StartXmlResourceRule.cs) |
| SWF 响应采集 | `AolaSwfCaptureSession.TryStart` 由单独的本地标记启用。通过 CEF 响应过滤器保存 SWF 字节和请求记录，用于资源调查。实现在 C#。 [采集入口](analysis/AolaLoader/ResourceInterception/Cef/AolaSwfCaptureSession.cs) |

## 核对范围

本次进行了 C# 入口、资源路由、战斗清单及主要 SWF 的静态核对。 `UseSkill.handleMovie`、多段浮字及面板采集等方法以 FFDec 的 p-code 为证。跨版本加载和实机效果的验证状态见各模块记录。 [项目约束](PROJECT_CONTEXT.md) · [战斗验证记录](doc/modules/BATTLE_SPEC.md) · [主题验证记录](doc/modules/THEME_SPEC.md)
