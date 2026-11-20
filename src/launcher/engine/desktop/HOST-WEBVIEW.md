# WebView2 原生宿主重建

`src/aoqi_web.rs` 是依据已恢复 `aoqi/platform/WebView.as` 契约重新实现的 Rust 模块，不是原 EXE 的 Rust 源码。它在 winit 父窗口内创建真实 WebView2 子窗口，Windows 负责网页输入、鼠标、滚轮和输入法。

## 固定依赖与集成

本模块仅用于 Windows，在父模块声明处使用 `#[cfg(windows)]`。已按官方 `wry-v0.53.5` 源码和 `webview2-com 0.38.0` 生成接口核对以下依赖：

```toml
wry = { version = "=0.53.5", default-features = false, features = ["os-webview"] }
webview2-com = "0.38"
windows = { version = "0.61", features = ["Win32_Foundation", "Win32_System_Com"] }
serde_json = "1"
```

所有 `WebViews` 操作必须在窗口事件循环线程运行。创建 WebView2 会泵送 Windows 消息，因此调用 `execute()` 时必须已释放 Ruffle 玩家和宿主状态锁。关闭父窗口前先 `dispose_all()`。传给 `new()` 的 profile 路径必须是应用工作区内的绝对目录；所有子视图共用该隔离 profile，代码不会读取系统浏览器 profile。

`web.create` 由父桥接层分配 ID、登记状态，然后排队 `Create`。新视图隐藏，正尺寸 `Frame` 后显示；零或负尺寸隐藏；`Dispose` 关闭控制器并销毁子 HWND。`Frame` 使用父窗口客户区 DIP；父层负责将 Flash 舞台坐标按 viewport 缩放、留边和菜单高度转换。窗口 DPI 变化后可调用 `resize()` 重设现有矩形。

## 契约与事件

| 命令 / 事件 | 实现和数据 |
| --- | --- |
| `Load` / `web.load` | WebView2 导航到传入 URL。 |
| `Run` / `web.run` | `ExecuteScript`，异步 HRESULT 失败上报 `failed`；JavaScript 自身抛异常并不保证产生 HRESULT 失败。 |
| `Post` / `web.post` | 原生 `PostWebMessageAsString`；页面 `chrome.webview` 的 `message` 事件收到原始字符串。 |
| `PostJson` | 原生 `PostWebMessageAsJson`，供恢复的 HTML 面板使用，保留对象类型；不改变 AS `web.post`。 |
| `message` | 原生 `WebMessageReceived`；AS 发布的字符串和面板发布的对象都保留类型。 |
| `navigated` | `SourceChanged`，载荷为当前 URL 字符串，包含同文档 URL 变化。父层先更新 `web.url` 缓存，再向 AS 派发。 |
| `loaded` | `NavigationCompleted.IsSuccess == true`，载荷为 URL 字符串。 |
| `failed` | 导航失败用 `WebErrorStatus` 整数；脚本原生失败用 HRESULT；同步创建或命令失败用明确约定的 `-1`，并返回 `Err` 供记录。 |
| `title` | 文档标题字符串；父层更新 `web.title`，不派发不存在的 AS 事件。 |

不能用 Wry `PageLoadEvent::Finished` 判断成功：该版本 Windows 实现不检查 `IsSuccess`。本模块直接检查 WebView2 的导航完成事件，失败不会被当作 `loaded`。

## 同步取消导航

父层必须安装 `set_navigation_handler(Rc<dyn Fn(&str, &str) -> bool>)`。回调中的 `true` 表示**允许**，而恢复的 `hostDispatch("web:" + id, "navigating", url)` 返回 `true` 表示**取消**，所以父层需要取反。回调应同步进入 AS，且不能在持有玩家锁时启动 WebView 操作。AS 回调产生的下一批原生命令仍排队，回调返回后再处理。

安装回调后不会另发异步 `navigating`，避免 AS 收到两次事件。没有安装时只产生观察通知并允许导航；这种配置不能声称支持 `preventDefault()`。被同步策略主动取消的导航按 NavigationId 识别，忽略其完成失败事件，避免误触发 `LoginOAuth.failed()`。

## 离线验收页面

`tests/fixtures/webview-bridge.html` 无外部资源和网络依赖。它支持真实 WebView2 连通检测、字符串与对象双向消息、原生脚本执行、中文及多行输入、滚轮、允许 / 取消本地导航和本地文件加载失败。

父宿主收到 `fixture.echo` 后使用 `Post` 或 `PostJson` 原样回传，页面会记录收到的数据类型。执行 `window.aoqiFixture.markRun("native-run-ok")` 可验证 `Run`；`window.aoqiFixture.snapshot()` 可供诊断读取。同步导航策略拒绝查询参数 `navigation=cancel` 可验证地址保持不变；缺失文件链接可验证 `failed` 而不是 `loaded`。这些是验收入口，不代表已经运行通过；实际编译和真机运行结果由主验收报告记录。

## 原始接口依据

- [Wry 0.53.5 WebViewBuilder API](https://docs.rs/wry/0.53.5/x86_64-pc-windows-msvc/wry/struct.WebViewBuilder.html)
- [Wry 0.53.5 Windows 源码](https://github.com/tauri-apps/wry/blob/wry-v0.53.5/src/webview2/mod.rs)
- [WebView2 NavigationCompleted](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2navigationcompletedeventargs)
- 本地契约：`flash/src/modules/host/scripts/aoqi/platform/WebView.as` 与 `WebViewEvent.as`，以及 `aoqi/LoginOAuth.as` 的 `preventDefault()` 调用。

## Shell 状态与面板集成

`src/aoqi_shell.rs` 维护 `WindowChrome` 的原生侧状态与队列。`Shell::call(name,args,storage)` 返回桥接值及 `ShellAction`，父事件循环执行真实窗口、面板和对话框动作。`ShellAction::Dispatch` 应调用 `hostDispatch("shell", kind, payload)`；不要在持有宿主状态锁时回调 AS。`drainCommands` 一次取走全部待处理命令，保留页面提供的命令 ID，并补充缺失的时间戳。

原 HTML 面板使用 `PostJson` 接收对象。父层把面板 WebEvent 的 `message.payload` 交给 `panel_message()`，再执行返回动作；初始化时发送设置、账号、关于的原生标签声明，以及已经收到的 AS 状态，并要求核心发布完整面板。`settings.*` 控件按恢复的 `ShellSettings` 允许值构建，未收到 AS 状态时保持禁用。账号表只显示 AS 的 `accountState().entries`；操作继续交给恢复的 AS 执行。当前重建账号标签提供现有账号选择、登录、删除和官方登录入口；添加、批量导入、密码编辑的原面板布局未恢复，本模块未伪造这些未知界面。

`ConfirmPanel` 的用户选择通过 `panel_confirm(request,confirmed)` 回传。`ShowDialog` 使用恢复的 `key/token/labels` 数据；父对话框完成后调用 `dialog_result(key,token,index)`，关闭可使用 `-1`。替换后的旧 token 和重复结果均被拒绝。`CloseDialog` 必须真实关闭对应 UI，不能仅删除状态。

`panelPrefs` 的 key 作为 JSON 属性保存于应用隔离存储 `native-panel-prefs.json`，不是文件路径；读到损坏 JSON 时返回错误并保留原文件。`inspect/payloadState/processMemory/rendererMemory` 由父层优先提供真实观测值，Shell 不填入虚构尺寸和内存。`updateState` 明确报告更新源未配置，`UpdateCheck` 动作应向用户呈现此状态。`FadeExit.seconds` 的依据是恢复的 `shell/Main.as:650`：`fadeExit(EXIT_FADE_MS / 1000)`。

本模块的编译核对已通过：本地 Rust 1.99.1、真实 release 依赖、`aoqi_web.rs` metadata 编译，以及 `aoqi_shell.rs + aoqi_storage.rs` metadata 编译。独立可执行验收使用真实 Storage，在工作区临时目录中验证了命令只派发一次、旧对话框 token / 重复结果拒绝、偏好跨 Shell 实例保留、损坏偏好不被覆盖，共 3 组通过。离线 HTML 的内联脚本通过 `node --check`。这些证据不替代父宿主的真机窗口 / WebView2 验收。
