# PARITY.md — 行为等价契约清单（M0 冻结）

- 文档版本：v1.0
- 日期：2026-11-22
- 状态：M0 产出，**M1 实现的验收依据**；本文件不改变现有 PowerShell 行为
- 上游计划：[rust开发.md](../plan/plan.md)（v1.2）
- 函数级清单：[parity-functions.md](parity-functions.md)（193 个函数，生成物）
- 黄金样本：[../fixtures/](../fixtures/README.md)

## 0. 用法

1. **M1 每完成一个模块**：找到本文件中归属该模块的契约，用「验证」列的样本跑通，再勾掉 [rust开发.md](../plan/plan.md) 对应任务。
2. **遇到本文件没写的分支**：先补契约条目，再写代码；不允许"实现随缘、事后补文档"。
3. **偏离既有实现必须有条目**：任何与现有 PS 不一致的行为，都写成 §4 的 `D-n` 条目，附理由与风险；没有 `D-n` 编号的差异一律视为缺陷。

## 1. 结论摘要

| 项 | 结论 |
|---|---|
| 现有实现规模 | 20 个 PS1、5,190 行、**193 个函数**；三家游戏脚本间 **20 组重名函数，其中 9 组正文不一致** |
| 必须逐字节一致 | 诊断 INI 文本、runtime 副本（marker + CRLF + 源文本、UTF-8 无 BOM、只读）、`recovered/<Session>-Engine.ini` 命名 |
| 只需字段级一致 | `active.json`（PS 的 `ConvertTo-Json` 缩进形态不复刻，见 D-1） |
| 已有样本 | 33 个文件：7 份模板 + 1 份派生模板 + 12 份参数矩阵 + 8 条识别用例 + 7 个 journal 阶段 + 13 条路径用例 |
| 有意偏离 | 10 条（D-1 ~ D-10），集中在"输出格式""默认值"与"分包字段"，不涉及恢复语义 |
| 未验证 | 6 项，见 §6 |

## 2. 契约清单

「现有实现」列是 Read 过的真实代码位置；「验证」列的样本路径相对 `rust/fixtures/`。

### C1 · journal schema v1

> 阶段取值：`deploying` / `recovering` / **`retained`**（三款 Run 脚本在 `-KeepLinks` 之后写入）。
> M6-01 用真实记录 `启动脚本\.state\steam-4356430\active.json` 验证过：缺 `retained` 会导致旧记录读不出来。

| 项 | 要求 |
|---|---|
| 文件 | `.state/<gameId>/active.json` |
| 必填字段 | `Version=1`、`GameId`、`Session`（32 位小写十六进制）、`ScriptPid`、`ScriptStarted`（ISO-8601 UTC，`o` 格式）、`Render`、`Engine`、`Runtime`、`Backup`、`OriginalExisted`、`OriginalHash`、`StorageExisted`、`Phase`、`Operations[]`、`Reused[]` |
| 条件字段 | `DeploymentMode="DlssgOnly"`（仅 DLSSG 模式）、`ProxyName`（可选代理时） |
| 操作项 | `{Kind, Path, Target, Hash, State}`；`Kind ∈ {link, engineLink, backup, runtime, storage, diagnostic, directory}`；`State ∈ {intent, done, restored}` |
| 阶段 | `Phase ∈ {deploying, recovering}`；新建时为 `deploying`，恢复开始时置 `recovering` |
| 写入 | 追加/更新后**每次**落盘；临时文件 + `ReplaceFile`；UTF-8 无 BOM |
| 空值 | `OriginalHash` 在原件不存在时为 `null`，不是空串 |
| 现有实现 | `DeploymentJournal.ps1:184-205`（创建）、`:28-49`（落盘）、`:245-259`（操作）、`:207-210`（阶段） |
| 验证 | `journal/01-new.json`、`02-runtime-done.json`、`03-engine-link-intent.json`、`04-partial-failure.json`、`07-no-original.json` |
| 格式边界 | **字段级等价**，见 D-1 |

### C2 · 互斥名与 identity

| 互斥 | 名称 | identity 算法 | 等待 |
|---|---|---|---|
| 部署/恢复 | `Global\DLSSG-Deployment-<identity>` | `UPPERHEX(SHA256(UTF8(GetFullPath(renderDir).TrimEnd('\').ToLowerInvariant())))` | `WaitOne(0)`，抢不到立即报错 |
| 配置写入 | `Global\DLSSG-Config-<identity>` | 同样的归一化和哈希算法，输入为实际 `Engine.ini` 路径 | `WaitOne(0)` |
| `启动配置.ini` 写回 | `Local\DLSSG-LauncherConfig-<id>` | `UPPERHEX(SHA256(UTF8(GetFullPath(iniPath).ToLowerInvariant())))`，**不 TrimEnd** | `WaitOne(5000)`，超时报错 |

- `AbandonedMutexException` 一律视为"已获得"。
- 新增：单会话互斥 `Global\DlssgSession`（N-1），与上面三个并存。
- 现有实现：`DeploymentJournal.ps1:72-79`、`:96-102`；`LauncherConfig.ps1:175-182`。
- 验证：M1 单元测试直接用固定输入比对字符串（identity 是纯函数）。

### C3 · 恢复顺序

`engineLink → backup → runtime → storage → link → diagnostic → directory`，**同类内逆序**（后建先拆）。

- 现有实现：`DeploymentJournal.ps1:406-411`。
- 验证：M1 用 7 类操作各 2 条记录，断言处理顺序。

### C4 · 依赖保留与冲突保留

恢复时若依赖未完成，保留依赖文件并继续无关步骤，**不删记录**：

| 场景 | 处理 | 现有实现 |
|---|---|---|
| `engineLink` 未恢复 | 不处理 `backup`/`runtime`（抛"dependent files preserved"） | `:419` |
| `backup` 未恢复 | 不处理 `runtime` | `:420` |
| `diagnostic` 目标仍被 `link` 引用 | 不删该临时文件 | `:422` |
| 任一单步冲突 | 记录冲突、继续其余步骤、最终报告失败 | `:412-499` |

### C5 · 被替换 `Engine.ini` 的归档

| 项 | 要求 |
|---|---|
| 触发 | `engineLink` 操作为 `done`，且目标路径变成**普通文件**（非重解析点、非目录） |
| 目标 | `.state/<gameId>/recovered/<Session>-Engine.ini` |
| Session 校验 | 必须匹配 `^[a-fA-F0-9]{32}$`，否则报"Invalid recovery session" |
| 冲突 | 归档目标已存在 → 抛错，**两边都保留**，不动原文件 |
| 之后 | 继续用备份恢复原件 |
| 现有实现 | `DeploymentJournal.ps1:438-450` |
| 验证 | `journal/06-recovered-archive.json`（链接状态为模拟，见 §6-1） |

### C6 · 资源、代理与 `_storage_`

| 项 | 要求 |
|---|---|
| 代理白名单 | 严格 6 个：`version.dll`（资源根）、`winmm.dll`/`dbghelp.dll`/`dinput8.dll`/`dxgi.dll`/`d3d12.dll`（`alternatives\`）；只部署一个，不自动轮换 |
| 资源链接 | 5 个目录软链接 `runtime`/`backend`/`kernels`/`xefg`/`sr` + 代理软链接 |
| 断言 | `dlssg_sm86.ini`、`runtime\nvngx_dlssg.dll`、`backend\sm86_backend.dll`、`kernels\fg_kernels.dll`、`xefg\efg_kernels.dll`、`sr\sr_kernels.dll`、`sr\nvngx_dlss.dll` 逐个 `throw`；`sr` 分层见 N-5 |
| `_storage_` | 副本按 **SHA256 与项目代理一致**才认领；不一致保留并告警 |
| 第三方文件 | 其他代理名的普通文件（mod 注入器）**记录后共存**，不修改不删除 |
| 删除 | 只删链接本身；资源根目录仅当为空时删除；禁止递归穿透链接 |
| 现有实现 | `LauncherResources.ps1:64-145`、`Run-WutheringWaves-DLSSG.ps1:348-460` |
| 验证 | **缺实链样本**，见 §6-1 |

### C7 · 诊断 INI（逐字节）

| 项 | 要求 |
|---|---|
| 来源 | 每个入口的 0.4.2 模板（`$diagnosticIni` here-string） |
| 替换规则 | 纯字面 `String.Replace`，顺序固定：`Optimized=2` → `Level=2` → `Preset=A` → `[SuperResolution]\nMode=Off` → `[SuperResolution]\r\nMode=Off` |
| 值域 | `SuperResolutionMode ∈ {Off, M-INT8, L-INT8, M-FP16, L-FP16}`、`DlssgPreset ∈ {A,B,C}`、`Optimized/Level ∈ {0,1,2,3}`；非法值在写入前就被拒绝 |
| 识别（无记录回收） | 提取 4 个值 → 用**同一模板**重新生成 → 与整段文本 `-ceq` 比较；任何多余 section、重复键、注释改动、行尾空格、LF 换行都会失败 |
| 旧文本白名单 | 鸣潮：0.4.2 / 0.4.0 / 0.4.0-Mode=Off / 0.3.2；异环：0.4.2 / 0.4.0；Steam/Direct：0.4.2 / 0.4.0 |
| 临时文件名 | 鸣潮 `dlssg_sm86-wuthering-<pid>.ini`、异环 `dlssg_sm86-nte-<pid>.ini`、Steam/Direct `dlssg_sm86-<gameId>-<pid>.ini`，位于 `%TEMP%` |
| 现有实现 | `LauncherConfig.ps1:377-421`；模板见 `Run-*.ps1`（鸣潮 `:110`、异环 `:78`、Steam `:108`） |
| 验证 | `templates/**`、`diagnostic/*/*.ini`（12 份）、`diagnostic/recognition.json`（8 条） |

### C8 · runtime 副本（逐字节）

| 项 | 要求 |
|---|---|
| 内容 | `<marker> + CRLF + 配置源原文` |
| 编码 | UTF-8 无 BOM |
| 属性 | 只读（`FileAttributes.ReadOnly`） |
| marker | 鸣潮 `; WuWa-Configs launcher runtime copy v1`；异环 `; NTE launcher runtime copy v1`；Steam `; Steam UE5 launcher runtime copy v1 appid=<AppId>`；Direct `; Direct UE launcher runtime copy v1 game=<gameId>` |
| 现有实现 | `Run-WutheringWaves-DLSSG.ps1:55,275`、`Run-NevernessToEverness-DLSSG.ps1:34,231`、`Run-SteamUE5-DLSSG.ps1:61,253` |
| 验证 | M1 用 fixture 模板 + 固定源文本断言前 64 字节；marker 逐字 |

### C9 · 会话策略（三家必须保持不同）

| 入口 | 策略 |
|---|---|
| 鸣潮 | 游戏进程退出后等待热重启窗口（默认 10 秒，可配置），窗口内重启则继续等待 |
| 异环 | 首次游戏进程退出即恢复 |
| Steam / Direct | 等待**全部**目标进程退出；启动最多 120 秒，超时正常恢复；退出后再等 3 秒关闭宽限 |

Rust 会话按完整渲染 EXE 路径观察。Steam、Epic 和通用 Direct 启动上限为 120 秒。
手动恢复同时检查独立官方启动器。无法核验同名候选路径时保留部署。新记录的可选
`Processes` 字段保存本次目标，旧记录通过原档案补足。中止信号仅交给对应的部署会话。

Rust 界面给每次引擎交接分配请求 ID，子进程报告同时携带请求 ID、游戏 ID、操作和 PID。
启动确认在部署与启动请求发出后单独写入；界面凭本次确认决定是否收进托盘。
操作结论以本次报告和子进程退出状态共同判定。普通 CLI 的报告格式保持兼容。

- 现有实现：`Run-*.ps1` 的主循环 + `LauncherSession.ps1`。
- 约束：**不允许**为了复用而统一这三条策略（仓库 `AGENTS.md` 明文要求）。

### C10 · `启动配置.ini`

| 项 | 要求 |
|---|---|
| 编码 | UTF-8 / UTF-8 BOM / UTF-16 LE/BE / UTF-32 LE/BE 自动识别；BOM 判定顺序 UTF-32 先于 UTF-16；非 UTF-8 的 GBK 等给出告警而不是猜测 |
| 键名 | 中文键 + 英文别名并存；`键=值`，**不支持行尾注释** |
| 报错 | 带行号 |
| 路径语法 | `%VAR%`、`$env:NAME`、`~`、`FileSystem::`、`\\?\`、UNC、`file://` URI |
| 写回 | 首次发现时写回，**保留原编码与 BOM**；写回走 `Local\DLSSG-LauncherConfig-*` 互斥 |
| 现有实现 | `LauncherConfig.ps1:6-…`（编码）、`:160-230`（写回） |
| 验证 | M1 用 6 种编码的固定样本断言读→写回字节 |

### C11 · 偏好目录

| 项 | 要求 |
|---|---|
| 路径 | `%LOCALAPPDATA%\DlssgSteamLauncher\<16hex>` |
| `16hex` | `UPPERHEX(SHA256(UTF8(GetFullPath(root).ToLowerInvariant())))[0..16]`（取前 16 个十六进制字符） |
| 结构 | `preferences.json`、`runs/<guid>/request.json`、`result.json` |
| 现有实现 | `PortableLauncher.ps1:5-11`、`:325-337` |
| 说明 | 新程序改名 `DlssgLauncher`，旧目录只读兼容（D-3） |

### C12 · 日志

| 项 | 要求 |
|---|---|
| 目录 | `<启动器根>\logs` |
| 文件 | `*.log`（每入口一份按日文件） |
| 清理 | 启动时删除 `LastWriteTime` 早于 24 小时、且**非重解析点**的普通 `*.log`；被占用/删除失败一律忽略 |
| 组件日志 | 只读，位于游戏目录 `dlssg_sm86\logs\dlssg_sm86.log`，按 EXE 名或 pid 过滤 |
| 现有实现 | `ScriptLogging.ps1:2-12` |

### C13 · 恢复拒绝条件

| 模式 | 条件 |
|---|---|
| 自动恢复（正常退出） | 只要求**游戏**已退出；官方启动器允许仍在运行 |
| 显式恢复（手动/崩溃后） | 游戏**和**启动器都必须已退出 |
| Steam/Direct | 只按完整渲染 EXE 路径判定；Steam 本身允许运行 |
| 拒绝时 | 保留现场与记录，返回失败；报错文案 `Game still running; deployment preserved.` / `Close <name> before recovery.` |

- 现有实现：`DeploymentJournal.ps1:381-401`。

### C14 · 便携包清单（M5-02 已实现：`launcher-core/src/export.rs`）

| 项 | 要求 |
|---|---|
| 文件 | `package-manifest.json`，`FormatVersion=1`、`PackageType`、`Files[]` |
| 自身 | 清单不包含自己 |
| 现有实现 | `Export-PortableLauncher.ps1:234-242` |
| 变更 | 文件集合按 `rust开发.md` §12.4 重写；`PackageType` 语义改为 `1=基础包 / 2=sr 配件`（N-6） |

### 本版新增规则

| ID | 规则 | 与现有实现的关系 |
|---|---|---|
| N-1 | 单会话 `Global\DlssgSession`，第二款游戏点启动时明确拒绝 | 新增 |
| N-2 | 复用已在运行的官方启动器，按完整 EXE 路径核对；游戏已在运行仍拒绝 | 新增（PS 只按进程名判断是否已启动） |
| N-3 | 生成的运行时 INI 带身份标记行 `; dlssg-launcher-config=<uuid>` | 新增，用于无记录回收 |
| N-4 | 档案字段 `EngineHost ∈ {none, UE4, UE5}`；`none` 不托管 `Engine.ini` | 替换 UE 自动识别（D-4） |
| N-5 | `sr` 分层断言：仅 `SuperResolution.Mode ≠ Off` 要求 `sr\*` | 放宽现有硬断言（D-5） |
| N-6 | 便携包 `PackageType` 取值改为 `1=基础包 / 2=sr 配件` | 旧 0–4 废弃 |
| N-7 | 运行时 INI 生成**完整 12 键** | 现有模板只有 8 键（D-2） |
| N-8 | 默认值 `MaxGeneratedFrames=3`、`Optimized=1`、`Level=1` | 现有模板是 5 / 2 / 2（D-2） |
| N-9 | 平台游戏必须由平台拉起：`launch_mode=Steam` 必须解析出 `Steam.exe` 与合法 AppID（缺 AppID 拒绝规划，不退回直接起 EXE）；扫描与手动添加都把 Steam 库里的目录写成 `steam.exe -applaunch <AppID>` | 新增（旧实现缺 `launch_executable` 时退回直接起渲染 EXE；Steamworks 游戏会自己要求 Steam 重启，部署在真正在跑的进程出现前就被恢复） |
| N-10 | 平台游戏必须由平台拉起（Epic）：`launch_mode=Epic` 必须解析出 `AppName`，启动走 `com.epicgames.launcher://apps/<AppName>?action=launch&silent=true`（ShellExecuteW），缺 AppName 拒绝规划；扫描与手动添加按 `LauncherInstalled.dat` 写 AppName；GOG 保持 Direct（DRM-free，直接起 EXE 是正常方式） | 新增（同 N-9：直接起 EXE 会绕开 Epic 的归属校验/在线服务） |

## 3. 三家游戏脚本的重复函数：不能盲目合并

`parity-functions.md` §2 列出 20 组重名函数，其中 **9 组正文不一致**。以下三组的差异会直接影响行为，M1 必须逐条决定"保留哪一支 / 参数化"：

### 3.1 `Prepare-EngineConfig`（49–50 行 × 3）

| 入口 | 差异 |
|---|---|
| 鸣潮 | `Assert-NoGameProcess`（只按进程名） |
| 异环 | 分别 `Assert-NoRunningProcess -Name $launcherProcessName -Description 'NTE launcher'` 与 `-Description 'NTE game'` |
| Steam/Direct | 先 `if (Test-JournalDlssgOnly) { return }`，再 `Assert-SteamGameStopped -Executable $gameExe`（按完整路径） |

Rust 侧：统一成"按档案声明的判定方式"，由档案字段选择 `ByName` / `ByExecutable`，不能在核心库里写三家分支。

### 3.2 `Assert-LinkOrAbsent`（30 行 vs 36 行）

| 入口 | 差异 |
|---|---|
| 鸣潮 | 额外允许把**指向仓库根 `dlssg_sm86.ini` 的旧链接**迁移为本次临时配置（`$projectIni` 分支，日志"仓库文件保持不变"） |
| 异环 / Steam | 无该分支；改为先调 `Test-StaleDiagnosticIniTarget` 判定过期临时链接并清理 |

Rust 侧：两支都要有——鸣潮的迁移分支是历史兼容，异环/Steam 的清理分支是新路径；用"允许的目标集合"参数化，不要合成一段 if-else。

### 3.3 `Test-StaleDiagnosticIniTarget`（11–12 行 × 3）

| 入口 | 文件名正则 | 接受的旧文本 |
|---|---|---|
| 鸣潮 | `^dlssg_sm86-wuthering-\d+\.ini$` | 0.4.2 模板 + `$diagnosticIni040` + `$legacyDiagnosticIni`（0.3.2）+ `$previousDiagnosticIni`（0.4.0-Mode=Off） |
| 异环 | `^dlssg_sm86-nte-\d+\.ini$` | 0.4.2 模板 + `$diagnosticIni040` |
| Steam/Direct | `^dlssg_sm86-<gameId>-\d+\.ini$`（`$gameId` 走 `[regex]::Escape`） | 0.4.2 模板 + `$diagnosticIni040`（先判 0.4.0，再判模板） |

Rust 侧：按 `gameId` 声明"临时文件名前缀 + 接受的旧文本集合"，模板全部内置（PARITY §2 C7）。

其余 6 组不一致（`Refresh-Game`、`Save-Editors`、`Set-Working`、`Show-Failure`、`Start-Action`、`Remove-GeneratedStorageDll`）属 UI 与清理逻辑，M4/M1 时逐条比对。

## 4. 与现有实现的有意偏离

> 没有编号的差异一律按缺陷处理。

| ID | 偏离 | 理由 | 风险与验证 |
|---|---|---|---|
| D-1 | `active.json` 只保证**字段级**等价，不复刻 PS `ConvertTo-Json -Depth 12` 的缩进（`"Key":  value` 双空格、4 空格层级、空数组跨行） | 该文件只由本程序读写；复刻 PS 序列化形态没有收益，反而限制实现 | 旧 PS 不读新记录（单向兼容，`rust开发.md` §2.9）；fixtures 保留 PS 实际形态作参考 |
| D-2 | 运行时 INI 改为完整 12 键，默认值 `3 / 1 / 1`（现有模板 8 键、`5 / 2 / 2`） | 与 0.4.2 发行配置 `dlssg_sm86.ini` 对齐；缺键依赖默认值会随上游版本漂移 | **行为变化**：6X→4X、有损档→逐位一致档。M6 真机确认画面/性能；M1 用 golden 锁死文本 |
| D-3 | 偏好目录改名 `%LOCALAPPDATA%\DlssgLauncher\<16hex>` | 程序更名，避免与旧 WinForms 入口互相覆盖 | 旧 `DlssgSteamLauncher` 只读兼容；首次启动可提示一次性迁移 |
| D-4 | 删除 UE 自动识别（C# `Add-Type` 探测、后台任务、冲突校准 UI） | 见 `rust开发.md` 2.5：靠声明不靠探测 | 填错 UE 版本无兜底；界面需明示 |
| D-5 | `sr` 资源按需（`Mode=Off` 允许缺失） | 172 MB 不值得为不用超分的人承担 | 组件在 `Mode=Off` 时是否仍探测 `sr` 文件**未验证**（§6-3） |
| D-6 | 单会话（`Global\DlssgSession`） | 见 `rust开发.md` 2.3 | 同时开两款游戏的用法失效，需明确提示 |
| D-7 | 便携包不再含 `.ps1`/`.bat`，`PackageType` 语义改为基础包/配件 | 入口合并为一个 EXE | 旧包脚本不可用；说明需重写 |
| D-8 | 恢复时旧 PS 记录里的 0.4.2 诊断 INI **按旧模板识别**，新生成用 12 键 | 旧链接必须能回收 | 识别集合必须包含 `templates/**` 全部 8 份模板
| D-9 | Rust 引擎在持锁状态下调用 `resources::assert_resources_removed`，通过后才删除实际命中的记录 | 手动恢复、自动恢复与回滚共用收尾流程，失败保留记录 | `step10_11_restore.rs` 与引擎恢复回归用例 |
| D-10 | `package-manifest.json` 每个条目多一个 `Package`（base / sr）字段，其余字段与 PS 完全一致 | 便携包按"基础包 + sr 配件"拆发；旧工具只用 `Path/Length/SHA256`，不会误解 | `export::tests` 校验键名与往返 |（C7） |

## 5. Golden 样本

目录结构、再生方式、归一化规则见 [../fixtures/README.md](../fixtures/README.md)。

| 样本 | 对应契约 | 用途 |
|---|---|---|
| `templates/wuthering/{0.4.2,0.4.0,0.4.0-mode-off,0.3.2}.ini` | C7 / D-8 | 无记录回收的精确文本 |
| `templates/neverness/{0.4.2,0.4.0}.ini`、`templates/steam-ue5/{0.4.2,0.4.0}.ini` | C7 | 同上 |
| `diagnostic/<game>/<combo>.ini`（12 份） | C7 | 3 款 × 4 组参数的生成结果 |
| `diagnostic/recognition.json` | C7 | 识别器的接受/拒绝边界 |
| `journal/0*.json` | C1 / C5 | 5 个阶段快照 + 归档结果 |
| `journal/06-recovered-archive.name.txt` | C5 | 归档命名模板 |
| `paths/normalized-path.json` | C2 / C3 | 路径归一化表达式与 13 条用例 |
| `manifest.json` | — | 每个样本的 SHA256、源文件哈希、生成时间与 Git 提交 |

## 6. 未验证清单（M1 前必须清零或明确接受）

| # | 未验证项 | 影响 | 何时补 |
|---|---|---|---|
| 1 | **真实软链接**相关的部署/恢复样本。当前会话无 `SeCreateSymbolicLinkPrivilege`，`journal/06` 用"记录为 done + 普通文件"模拟归档分支 | 链接创建/校验/拆除路径没有字节级样本 | 以管理员身份重跑 `gen-fixtures.ps1`，或 M1 集成测试里提权 |
| 2 | **UAC 提权隐藏子进程**：`spike --elevate` 未在自动化里执行（会弹 UAC 且无人确认） | 「GUI 非提权 + 引擎提权 + 无控制台」链路未端到端验证 | 手工执行一次并记录结果 |
| 3 | 组件在 `SuperResolution.Mode=Off` 时**是否仍探测** `dlssg_sm86\sr` 文件 | 决定 D-5 是否成立；不成立则 `sr` 必须随包（223 MB） | M6 真机 |
| 4 | 组件是否接受 12 键中的 `[XeSSFG]` 两键与 `[Runtime] Path`（在 `Engine=DLSS` 时） | D-2 的正确性；若被拒，日志应有 `configuration_invalid` | M6 真机 + 组件日志 |
| 5 | 旧 PS 是否真能读新记录 | 不在范围内（单向兼容），**不测** | — |
| 6 | 三家游戏真机加载、退出恢复、性能收益 | 契约之外的现场证据 | M6 |

## 7. 变更记录

| 版本 | 日期 | 说明 |
|---|---|---|
| v1.0 | 2026-11-22 | M0 首版：14 条兼容契约 + 8 条新增规则 + 3 组重复函数差异 + 8 条有意偏离 + 6 项未验证 |
| v1.1 | 2026-11-23 | 新增 N-9（平台游戏必须由平台拉起）：`launch_mode=Steam` 缺 AppID/`Steam.exe` 时拒绝规划；扫描/手动添加都按 Steam 清单写启动方式 |
| v1.2 | 2026-11-23 | 新增 N-10：Epic 档案按 `com.epicgames.launcher://` 拉起（缺 AppName 拒绝规划），扫描/手动添加按 `LauncherInstalled.dat` 识别；GOG 保持 Direct |
