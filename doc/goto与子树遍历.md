# goto与子树遍历

## 定位结果

`gotoAndStop → goto_frame → goto_frame_now → run_inner_goto_frame`会执行整个舞台与orphan的构建、脚本遍历和事件处理；同帧跳转也有可观察副作用。orphan指无父节点，不能等同于隐藏或屏外对象。

2026-11-09采集9次详情响应，每次约0.70～1.25秒。goto内部舞台构建/脚本遍历占51.98%，orphan构建/脚本遍历46.23%，合计98.20%；清理和实际帧脚本执行占比很小。2153次goto中73.34%为同帧，但这不是可直接消除的耗时比例。

逐响应统计独立于普通帧，`phases[].exclusiveMs`之和等于`outerTotalMs`；`inclusiveMs`包含嵌套重叠，不能再加。访问次数不是唯一对象数，槽位增长也不是泄漏证明。

## 本轮唯一优化

旧的`sleeping_subtree`只在orphan根入口使用；舞台或活跃父节点下面的停止分支仍会逐层调用`run_frame_scripts`。

在MovieClip脚本遍历入口复用整棵子树的休眠证明和已有唤醒机制。仅在新开关与sleep都启用时生效，默认关闭。构建、广播、同帧goto、移除对象列表和清理顺序未改；普通帧与嵌套goto均经过这一脚本入口。

源码：`src/optimization/engine/core/src/display_object/movie_clip.rs::run_frame_scripts`、`core/src/orphan_manager.rs::sleeping_subtree`、`core/src/goto_profile.rs`；测试：`core/src/player/aola_goto_tests.rs`。

## 验证与当前结论

- 62次回归执行通过：26项核心和1项睡眠唤醒在开关两态各一次，加8项桌面检查。6个Flash样例结合两种统计开关、两种优化开关，共24次输出一致。
- 离线8层静止子树样例：100次同帧goto，访问1000→100；子对象play后恢复到300。该90%仅是样例访问量，不是游戏FPS收益。
- 2026-11-10 B组9次响应，显示正常，开关确实启用，无响应丢失，阶段耗时完整对齐；最终记录00:13:11。
- 与历史未启用版比较：舞台脚本访问-40.57%，阶段耗时仅-2.32%；orphan脚本访问-3.19%，耗时+56.44%；详情总耗时+4.82%。两次对象规模、操作顺序和生命周期未对齐，**没有同包A组，整体净收益未证实**。
- 静止判定可能在活动分支上反复检查后代，抵消访问减少；日志没有独立判定计时，这仍是嫌疑。

证据：[实现交付](evidence/history/subtree-scripts-20261109/delivery.json)、[goto定位结果](evidence/history/goto-results-20261109/analysis.json)、[B组分析](evidence/history/subtree-results-20261110/analysis.json)。

## 当前测试操作

1. 关闭旧程序，在新目录运行[同包A控制入口](../dist/aola-subtree-scripts-test-20261109/run-subtree-A-control.cmd)，手动登录。
2. 与B保持相同地图、窗口和约8只亚比顺序，每只停约2秒；关闭背包等5秒后正常退出，记录结束时间和显示情况。
3. 需要重做B时运行[同包B优化入口](../dist/aola-subtree-scripts-test-20261109/run-subtree-B-optimized.cmd)。两个入口只有新优化开关及独立数据目录不同。
4. 日志分别在该包`aola-data-subtree-A/B/diagnostics`，schema 7中A的`subtreeScriptSleep=false`、B为true。
5. 用`src/tools/analyze-performance.py --a <A日志或diagnostics目录> --b <B日志或目录> --output <输出JSON>`复算。核对窗口、版本、响应数、丢弃数和结束标记，再比较每响应耗时、访问量和orphan规模。

历史B日志在`doc/evidence/runtime/aola-subtree-scripts-test-20261109/aola-data-subtree-B/diagnostics`，原登录资料未迁移。当前不叠加构建遍历或卡片统计优化，不直接跳过同帧goto。上一轮[定位版](../dist/aola-goto-profile-test-20261107/run-goto-profile.cmd)保留用于回退定位。
