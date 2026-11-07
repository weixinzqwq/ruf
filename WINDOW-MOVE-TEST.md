# Windows 窗口拖动修正版

交付目录：`dist/aola-window-move-test-20261106`。

本轮只修正宿主的窗口调度。Windows 进入拖动或缩放消息循环后，原有 `AboutToWait` 暂停派发，播放器原先唯一的 tick 入口因而停止。新增窗口消息钩子在该循环内启用 10 ms 唤醒定时器，经 winit 用户事件调用既有更新与绘制路径。该间隔是唤醒请求，不是改变游戏帧率；仍按同一 `Instant` 计算实际经过时间。

退出拖动时立即撤销定时器；窗口销毁或钩子释放也会清理。最多保留一个待处理唤醒，普通事件循环和拖动期间共享时间戳，避免重入堆积或重复推进。原有窗口消息继续交给 winit 与 WebView2。

## 手动检查

1. 正常退出旧测试版。使用本目录的 `run-detail-B-optimized.cmd` 启动并在官方页面登录。若上轮用 A，则继续用 `run-detail-A-original.cmd`；两者都有窗口修正，SWF 和启动配置均沿用上轮。
2. 在可见动画场景中，按住标题栏持续移动约 5 秒，再保持按住鼠标但停止移动约 5 秒。两种情况下画面均应继续更新。
3. 松开鼠标，确认没有停顿后突然快进；重复拖动几次。再拖动窗口边框调整大小，检查游戏画面、鼠标位置和登录页布局。
4. 可继续把 PresentMon 记录放到 `D:\game\Rea\aola\capture`。运行日志会记录 `Window move/size loop entered/exited`，退出记录中的 `ticks` 是该次拖动期间完成的更新次数。

新目录使用独立的数据目录；没有复制账号文件或启动登录会话。上一版 `aola-pet-detail-test-20261106` 保留。

本轮自动检查通过隐藏窗口中的原生嵌套消息循环验证事件调度、拖住不动、重复拖动、退出和清理；这不替代在线游戏及真实鼠标拖动的画面验收。Windows 定时器受系统调度影响，复杂场景自身的脚本或绘制耗时仍可能降低帧率。

## 源码与依据

- `desktop/src/window_move.rs`：定时唤醒、合并、生命周期及原生嵌套循环回归检查。
- `desktop/src/app.rs`：窗口守卫和拖动唤醒接入原有更新路径。
- 本轮不更改 SWF、渲染器、回收策略或强制帧率。
- Windows 消息行为：[WM_ENTERSIZEMOVE](https://learn.microsoft.com/en-us/windows/win32/winmsg/wm-entersizemove)、[SetTimer](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-settimer)、[SetWindowSubclass](https://learn.microsoft.com/en-us/windows/win32/api/commctrl/nf-commctrl-setwindowsubclass)。

构建：`tools/build.ps1 -Variant window-move`。
本轮源码备份、差异和检查输出：`evidence/window-move-20261106`。
