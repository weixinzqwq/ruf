# 已重建的 Rust 局部算法库

这里是可编译、可修改的 Rust 源码库，使用标准库，没有外部依赖。它根据发布版机器码重写了三个有限范围的模块，**不是完整的 Air EXE 工程，也没有恢复完整 Ruffle/辅助业务运行时**。API 和变量名称由恢复者命名；所有实现均与下面的静态证据建立对应关系，没有空业务方法。

目标 SHA-256：`cf96adee1525e048de162c2dc4914e4bb1de8943b1e8e045c215f40105be2b53`。

## 构建

在本目录、配置好 Rust 及 Windows MSVC 链接环境后执行：

```powershell
cargo test --offline
cargo build --release --offline
```

在上级workspace构建时，库文件为 `../target/release/libaoqi_recovered_core.rlib`；它需要由后续恢复的宿主引用，不会独立启动游戏。构建只执行本目录测试代码。

## 确认的范围

| 模块 | 实现 | 静态证据 |
|---|---|---|
| `src/dense_storage.rs` | 有效 discriminant 到紧凑 tag/payload 的编码；8 槽、72 字节块；原寻址写入 | `0x140ce83c0`，对应插入路径 `0x140ce8660`；两张原 PE 跳转表 `0x141fe8a04` / `0x141fe8a3c` |
| `src/slot_layout.rs` | `8*n + 8*ceil(n/8)` 的分配大小、8 字节对齐及分配上限检查 | `0x140c511e0`，算术与分配检查位于 `0x140c51224..0x140c5126a` |
| `src/runtime_config.rs` | `renderScale` 1..4、无效时1；GC slice 默认4、正有限值限幅0.1..12；GC sleep默认0.1、接受非负有限值 | `0x1404c7770`、`0x140ba78e0`、`0x140ba7b30` |

编码中的 `0x34..0x39`、`0..0x33`、`UINT64_MAX` 是本构建的原生表示，未擅自为其补造原 Rust 枚举名。普通值占16字节；紧凑块包含64字节payload及8字节tag。不要将这部分数据区节省比例解释为整个进程内存节省比例。

GC slice 的重要边界：0、负数、NaN、正负无穷回退 **4.0**，不会先被限成0.1。GC sleep接受正零与负零，保留原始零的符号，不对较大正数限幅。这两项由指令中的 IEEE-754 位检查恢复。这里仅实现已经解析成功的数值阶段，没有冒充环境变量字符串解析或 Windows 非Unicode环境值处理。

`render_scale(None)` 表示缺失或不合原数值类型的输入。调用者应仅将无符号整数表示传入 `Some`，不得先把字符串、浮点数或负整数强制转换后传入。

## 证据位置与验证

完整的既有恢复资料副本位于本库的 `../../recovery-materials/`。以下路径相对于该目录：

- `provenance/native-comparison/dense-storage-encoding.json`：原PE跳转表字节、摘要、有效输入编码表。
- `native/assembly/function-140ce83c0.asm.txt`：`0x140ce8432/0x140ce843a` 的低32/8位零扩展；`0x140ce8463..0x140ce8481` 的块与槽寻址。
- `native/assembly/function-140ce8660.asm.txt`：插入路径中相同的编码规则；完整容器扩容不在本库中。
- `native/assembly/function-140c511e0.asm.txt`：分配算术、字节数上限及对齐8。
- `native/assembly/function-1404c7770.asm.txt`：查找字符串`renderScale`的长度11、数值表示检查及范围选择。
- `native/assembly/function-140ba78e0.asm.txt`：`0x140ba7a1b..0x140ba7a8c` 的有效数过滤、回退与限幅。
- `native/assembly/function-140ba7b30.asm.txt`：`0x140ba7c6b..0x140ba7ccd` 的有效数过滤与回退。
- `provenance/native-comparison/PSEUDOCODE-REVIEW.zh-CN.md`：原PE常量的文件偏移；4.0=`0x1578010`，0.1=`0x1f39a30`，12.0=`0x1577e80`，均按f64读取。

10项Rust测试覆盖原跳转表的所有标量分支与52个对象类编码、72字节布局和tag偏移、跨块写入、逻辑长度边界、Slot分配的尾组和溢出、renderScale范围，以及GC数值的正负零/非正规数/有限极值/NaN/无穷边界。GC测试还将高层实现与按原指令转写的位判断对照，覆盖全部2048个指数、两种符号和四种尾数，共16,384个组合。

另外，`tools/verify_dense_machine.py` 使用已有Unicorn 2.1.2隔离仿真目标的`0x140ce83c0`，并与实际编译的Rust示例逐字节比较两个完整块（144字节）。59种有效discriminant × 3种payload × 3个索引（0、7、8），共531组通过，所有邻槽保持原样。脚本固定目标SHA-256，限制在所选函数内，禁止进入调用helper的路径，检查执行返回及指令/时间上限；不启动Windows程序、不登录或访问服务器。这是所选有效索引路径的机器码差分验证，不是整个程序动态等价验证。

复验（在本目录运行）：

```powershell
cargo build --release --offline --example dense_vectors
python .\tools\verify_dense_machine.py --target '..\..\Air奥奇传说1.0.3.exe'
```

默认报告为 `../evidence/dense-machine-verification.json`；可通过 `--probe` 指定Rust示例EXE、`--report`指定报告文件。此额外验证需要Python环境已安装Unicorn，普通Rust构建和单元测试不需要Python或Unicorn。

## 明确未恢复的部分

- DenseStorage原始Rust类型、GC trace/sweep、扩容、紧凑与普通表示转换、复制/析构、完整读取与插入方法。
- SlotStorage的实际初始化、对象/trait/vtable布局、借用计数与分配失败的原ABI。
- 环境变量/JSON完整解析器、持久化设置、默认启用状态、渲染器和GC调度器。
- Windows宿主/WebView2桥、游戏导航、完整辅助核心状态机、私有Ruffle改动、原Cargo.lock及构建配置。

本库对原始无效枚举表示返回`None`、对分配溢出返回`None`，属于明确的重建API选择；原程序的无效枚举行为未恢复，分配失败走panic/分配错误helper。`sentinel_filled()`是方便调用方建立已知编码的初始化工具，不声称原容器所有分配都如此初始化。不要把这些辅助接口当作原始ABI。
