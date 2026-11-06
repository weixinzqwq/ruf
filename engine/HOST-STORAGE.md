# 存储、恢复与辅助核心接入

本次新增文件为 `desktop/src/aoqi_storage.rs`、`aoqi_recovery.rs`、`aoqi_helper.rs`。保留 Rust 技术栈。宿主构建统一由父任务执行；本文件不把已写入测试等同于已通过测试。

## 存储与资源

`Storage::new(root: PathBuf) -> Result<Storage, String>` 创建并固定根目录。`call(&self, name: &str, args: &[Value]) -> Result<Value, String>` 覆盖已恢复 `Storage.as` 的全部 14 个调用名。

| 方法 | 参数与结果 | 证据/实现性质 |
|---|---|---|
| `storage.root` | 无参数；根目录字符串 | AS 契约恢复 |
| `storage.read` / `write` | 路径；读返回 Base64 或 null，写接收 Base64 并返回 true | AS 契约恢复 |
| `storage.readText` | 路径；UTF-8 文本或 null | AS 契约恢复；无效 UTF-8 显式失败 |
| `storage.writeText` | 路径、文本、append 布尔值；成功 true | AS 契约恢复 |
| `storage.info` | 路径；不存在时 `{exists:false}` | `exists` 为 AS 已确认字段；size/isDirectory/isFile/name/path/modified 为独立重建字段 |
| `storage.list` | 路径；按名称排序的直接子项字符串数组 | AS 仅确认 Array；子项结构未恢复，当前自主定义为名称 |
| `storage.makeFolder` / `remove` | 路径；成功 true | AS 契约恢复；remove 不允许根目录 |
| `storage.copy` / `move` | 源、目标路径；成功 true | AS 契约恢复；为避免丢数据，目标必须不存在；跨卷 move 失败而不隐式删源 |
| `storage.reveal` | 路径；`{action:"reveal",path}` | 操作由窗口事件循环执行，此模块不伪称 Explorer 已打开 |
| `storage.resource` / `resourceText` | 精确资源逻辑名；Base64/UTF-8 或 null | 使用 `aoqi_assets::get`，不从文件系统寻找替代资源 |

`read_json` / `write_json` 为 Rust 内部便利 API，不增加不存在的 Flash 原生方法。路径允许 AS 生成的根内绝对路径；拒绝根外路径、`..`、Windows 设备名、ADS、尾点/空格、符号链接与 reparse point。根目录本身不可替换/删除。目录删除在改动前遍历检查链接。

文件写入使用同目录独占临时文件，写入并同步后 `MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)`。此顺序对应原生 `0x140154520` 的 `FlushFileBuffers` 和 `MoveFileExW(..., 9)`，但 Rust 实现为重建。失败保留原目标并清理本次临时文件。

路径检查适用于独立应用数据目录；它并非对抗另一进程以管理员权限持续替换目录节点的文件系统隔离机制。宿主不应与未受信任进程共享数据根。

## 缓存迁移

`migrate_cache(source: &Path, destination: &Path) -> Result<Value, String>`：

1. 宿主先停止使用缓存的 WebView/资源写入器，并取得用户选择的新位置。函数不会选择或扫描任何真实用户缓存。
2. 两路径必须绝对、互不包含，目标必须不存在，其父目录必须存在。
3. 在目标父目录建立唯一 `.名称.incoming-*` 目录。逐文件复制、同步和 SHA-256 校验；空目录保留。
4. 完成后重新核对整个源目录与临时目录的清单、大小和摘要，防止复制期间修改源文件。
5. 将临时目录更名为新目标。只有成功返回后，宿主才把新缓存路径原子持久化并重新打开缓存。

返回报告包含 source/destination/files/bytes/sourceRetained/verified/completedAt。源目录在成功和失败时均保留。失败的 incoming 目录留作诊断，错误包含其准确路径；未完成目录绝不会被写入缓存位置配置。程序再次启动应继续使用旧路径。源/目标不可在迁移期间由外部程序并发写入；不存在声称跨进程绝对一致性的快照。

### 导航器实际消费缓存

`frontend-utils/src/backends/aoqi_cache.rs` 公开 `AoqiCache::new(root)` / `stats()`；`ExternalNavigatorBackend::with_aoqi_cache(root)` 将同一迁移目标接入真实 `fetch`。宿主应传入保存的 `root/cache`，磁盘格式在其 `v1` 子目录中。

仅缓存 100bt 静态二进制资源 GET（SWF、常见图片/音视频）。用户名密码 URL、账号/登录/API 路径、非版本类查询参数、凭据请求头、隐式 Cookie、POST、私有/禁止缓存/Set-Cookie/部分响应均跳过。查询键仅允许 v/ver/version/hash/rev，值限字母数字、点、下划线和短横线，不写请求头或 Cookie。正文还需匹配资源文件签名；HTML/JSON 登录回应不会因 URL 以 `.swf` 结尾而进入缓存。

导航器保持原有流式读取：完整 body 或最后一个 chunk 成功后才发布缓存；取消/中断下载不产生可命中项。读取时核对 URL、失效时间、SHA-256 和资源签名。服务器 max-age 优先；缺省版本资源 7 天，无版本资源 5 分钟。缓存错误回退正常网络行为，网络被禁用时有效缓存仍能命中。

`stats()` 实际扫描缓存目录，返回 entries/diskBytes/legacyFiles/legacyBytes。entries 是磁盘 `.cache` 文件数量，内容有效性在命中读取时验证。旧格式未获验证，未知旧文件保留并统计为 legacy，不冒称能够命中。原程序独立验收目录中指定的四处 Cache/game-files/cache 候选均不存在，未读取账号或 WebView 数据。

新增 5 组磁盘/策略测试，以及 2 组导航器测试：先通过响应读取捕获资源，复制 v1 到迁移目标，再把 HTTP client 设为 None 验证实际离线命中；并检查隐式 Cookie 绕过缓存、流未读完/失败不发布。测试不发网络请求；结果仍以主任务执行为准。

## 启动恢复

`StartupRecovery::begin(root: &Path, requested_safe_mode: bool)` 在 `.host-state` 下独占打开进程锁，并使用原子文件保存启动状态。

- 对象需持续存活到窗口事件循环退出。第二个同数据目录实例会明确失败，不覆盖现有状态。
- `safe_mode` / `previous_unclean` / `consecutive_failures` 可供宿主决定恢复界面；`summary()` 可用于诊断。
- 窗口、渲染器、原生桥和 host.swf 就绪后调用 `mark_ready()`。
- 正常关闭调用 `finish_clean()`；`Drop` 不写 clean，避免掩盖崩溃。
- 用户请求安全模式重启时调用 `prepare_restart(true)`，退出并释放锁后再启动新进程。safe-mode 请求只消费一次。
- 非正常退出、无法解析的标记触发安全模式。损坏标记另存原始字节；不删除账号、设置、WebView、shader 或游戏缓存。

状态文件格式和安全模式策略是独立重建，未声称与原 `startup_recovery.rs` 全量等价。安全模式具体禁用哪些运行功能由窗口宿主负责；本模块只负责检测与持久化。

## 辅助核心协议与范围

`Helper::default()`；`call(&mut self, request: &str) -> String` 接收 `{"protocol":1,"op":...,"args":[...]}`，返回 `{"value":...}` 或 `{"error":...}`。该边界来自 `HelperCore.as`。

| 操作 | 状态 |
|---|---|
| `core.protocol` | AS 确认常量 1 |
| `core.reset` | 重建，实际清理全部状态机，旧句柄失效 |
| `machine.create` / `dispose` | 重建 helper 类型的真实生命周期；未知类型明确失败 |
| `machine.input` / `answer` | 按 `HelperMachine.as` 实现 call/args → value/thrown 续接；错误不伪装成功 |
| `machine.view` | 重建 panel/schema3、status-bar、performance 视图；其它视图明确失败 |
| `loginRecovery.plan` | 按原生 `0x140377b40` 分支、准确字符串常量与返回键静态恢复；未进行原机差分/真实登录验收 |

重建 helper 输入支持 `command`、`tick`、`invalidate`、`dispose`、`observe`。命令支持 `panel-refresh`、重建 `helper.observe` 控件的 `panel-event`、`accounts-busy`、`account-switch`。初始化实际发出 `panel.snapshot` 和 `panel.prefs.read` 回调；偏好修改实际等待 `panel.prefs.write` 完成，失败不更新内存值。通信观察只存当前会话计数，不存请求内容或凭据。

原辅助机器的战斗、日常、兑换、录制、配置/规则系统等尚未恢复。未知命令、游戏事件、控件和视图返回明确错误；重建面板显示当前范围。这不代表原辅助业务完整实现。

`loginRecovery.plan` 只输出 wait/password/prompt 决策，不发网络请求。精确匹配的类名为 `mmo.login.login.panel::LoginWithUserInfoPanel`、`mmo.login.login.panel::LoginWithoutUserInfoPanel`、`mmo.login.panel::InvalidPanel_res`；required/allowed/busy/prompted/hasPassword/attempted/hasSession 严格按 JSON 布尔值处理。原文提示取自原 EXE VA `0x14159572a` / `0x1415957ae`，长度分别为 132 / 123 字节。

## 已写入验证

存储模块 6 组 Windows 可运行测试：文本/二进制/JSON/追加、越界与设备名、复制/移动/列表/资源、原子替换失败保留、迁移与源保留、修改摘要检测。Unix 额外包含链接测试。恢复模块 5 组测试：正常关闭、未标记 clean 的重启、一次性安全模式、并发实例锁、损坏标记保留。辅助核心 4 组测试：协议与句柄、真实初始化续接、偏好写入失败回滚、登录纯决策。

这些测试只创建独立临时目录和合成数据，不读真实账号或缓存，不启动游戏，不登录。编译/测试结果以主任务实际执行记录为准。
