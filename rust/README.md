# rust/ — Rust 单 EXE 启动器（v0.1，M1–M10 已完成）

界面 + 引擎都在一个 `DlssgLauncher.exe` 里：不带参数是单窗口界面，带 `--engine` 是引擎。
计划与任务清单见 [plan/plan.md](plan/plan.md)（源：`plan/plan.json`）。版本号取自
`crates/launcher/Cargo.toml`，`DlssgLauncher.exe --version --report out.json` 可查。

> 磁盘：`target/` 与 `spike/target/` 都不入库。跑过 `check.ps1` 之后
> `target/debug` 会长到几个 G（本次约 4.4 G），`cargo clean` 或直接删目录都能再长回来；
> `target/release` 只有约 660 MB，里面是发布用的 exe。

| 路径 | 内容 |
|---|---|
| `docs/PARITY.md` | 行为等价契约：14 条兼容契约 + 8 条新增规则 + 8 条有意偏离 |
| `docs/parity-functions.md` | 193 个 PS 函数的逐行映射（`tools/gen-parity-inventory.ps1` 生成，**不入库**） |
| `docs/M0-REPORT.md` | M0 门禁报告：样本状态、spike 实测、未达标项与选项 |
| `docs/PARITY-REPORT.md` | 等价报告：逐条契约的验证方式与剩余未知 |
| `docs/M4-REPORT.md` | 界面验收：内存/首帧/DPI 实测与测量方法 |
| `docs/M6-DIFF.md` | PS ↔ Rust 差分报告：21 项差异 0，及不可比项清单 |
| `docs/m4-ui-150.png` | 150% 缩放下的窗口截图 |
| `docs/phase1-progress.md` | Phase 1 进度与关卡 A–E 结论 |
| `plan/plan.json` | **机器可读计划**：任务、依赖、验收、规则、关卡（事实来源） |
| `plan/plan.md` | 计划的只读视图（`tools/gen-plan.ps1` 生成，**不入库**） |
| `fixtures/` | 51 个黄金样本 + 再生脚本说明（`-text` 保护，勿手改） |
| `tools/gen-fixtures.ps1` | 调用现有 PS 实现生成样本（只在 `%TEMP%` 写入） |
| `tools/gen-parity-inventory.ps1` | 重新生成函数级清单 |
| `tools/measure-spike.ps1` | spike 实测（冷启动 / 内存 / 无控制台 / DPI / 中文） |
| `tools/check.ps1` | 一键检查：fmt + clippy + test（`-IncludeIgnored` 跑真实链接用例） |
| `tools/gen-plan.ps1` | 由 `plan/plan.json` 生成 `plan/plan.md` |
| `tools/diff-parity.ps1` | 差分核对：PS 现算 vs 入库样本（诊断 INI 12 份 + 识别 9 条） |
| `tools/measure-ui.ps1` | 界面验收：3 次启动采样内存/首帧并截图 |
| `tools/shot-ui.ps1` | 开发期截图：按标题抓窗口（Per-Monitor V2 DPI、整屏裁切、可悬停触发提示） |
| `spike/` | egui + glow 的最小工程，只用于测指标，不进产品 |

## 运行根布局

正式包**只有一个 exe 在根目录**，DLL 与 INI 放进 `bin\`：

```
DlssgLauncher.exe
bin\dlssg_sm86.ini
bin\version.dll
bin\alternatives\{winmm,dbghelp,dinput8,dxgi,d3d12}.dll
bin\dlssg_sm86\{runtime,backend,kernels,xefg}\*.dll
bin\dlssg_sm86\sr\{sr_kernels,nvngx_dlss}.dll   ← 轻量版没有这一对
配置\虚幻通用分类\UE{4,5}\{01..05}\Engine.ini
配置\鸣潮\Engine.ini   配置\异环\Engine.ini
```

`bin\` 存在就把负载根切到它，不存在就用启动器根（旧布局照旧可用）。
`配置\`、`logs\`、`.state\` 始终跟着启动器根。恢复原状时按部署记录里的代理链接
反推负载根，所以换过布局也能正确收尾。

恢复优先读取运行根的 `.state`，可回退到旧的 `启动脚本\.state`。已命中的记录在原路径更新，
持有渲染目录部署锁和适用的 Engine.ini 配置锁，直到恢复和残留检查都通过才删除记录。
配置锁按实际 Engine.ini 路径计算，与 PowerShell 入口共用。部署或启动失败时，报告会同时包含
原始错误和回滚结果。回滚或残留检查失败时保留记录，处理冲突后可重试「恢复原状」。

新记录同时保存渲染 EXE 和独立官方启动器的完整路径。手动恢复会核验这些进程均已退出，
无法读取同名候选的路径时会保留部署。旧记录需要原游戏偏好来补足路径。

## 打包（全量 / 轻量）

```
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\pack.ps1 -Mode full  -Force -Compress max
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\pack.ps1 -Mode light -Force -Compress max
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\pack.ps1 -Mode sr    -Force -Compress max -ArchiveOnly
```

同一个 exe，两个包；差别只有 `bin\dlssg_sm86\sr`（`sr_kernels.dll` + `nvngx_dlss.dll`，共 172 MB）。

压缩方式三档（`-Compress`）：

| 档 | 做法 | 校验 |
|---|---|---|
| `none`（默认） | 只出目录 | — |
| `zip` | Deflate（Optimal），等价旧参数 `-Zip` | 逐条目大小 + SHA256 与清单比对 |
| `max`（极限） | **7z / LZMA2 等级 9**，走 `C:\Bandizip\bz.exe`（与旧脚本 `Export-PortableLauncher.ps1 -b` 同一套）；没有 Bandizip 时退 7z.exe → tar+xz → zip | `bz t` CRC 校验，先写 `.partial` 再改名 |

三份产物（`-Mode full|light|sr`），两种压缩都出过：

| 包 | 解压后 | zip | max（7z） |
|---|---|---|---|
| `DlssgLauncher-v0.1-full`（全量） | 219.1 MB / 27 文件 | 87.7 MB | **57.5 MB**（26%） |
| `DlssgLauncher-v0.1-light`（轻量） | 46.7 MB / 25 文件 | 24.4 MB | **18.8 MB**（40%） |
| `DlssgLauncher-v0.1-sr-patch`（超分补丁） | 172.4 MB / 3 文件 | 63.2 MB | **38.2 MB**（22%） |

**轻量 + 补丁 = 全量**：逐个文件比过 SHA256 完全一致；功能上也验过——合并后开
`SuperResolution.Mode=M-Int8` 跑 `--engine run --dry-run` 得 `preflight:ok, resources:5`，
没打补丁的轻量包同样条件会被预检拒绝（"缺超分内核"）。

体积上拆分基本不亏：`18.8 + 38.2 = 57.0 MB` 与全量的 57.5 MB 几乎相同，
但不需要超分的人只要下 18.8 MB。补丁的用法写在该包的 `README-超分补丁.txt` 里
（把 `bin\` 覆盖到启动器根目录即可）。

`-ArchiveOnly` 可在压缩成功后删掉解压目录，只留压缩包。

所有 7z 都做过解压后逐文件 SHA256 复核。

输出到 `rust\dist\`（已 gitignore），每个包里有 `package-manifest.json`（版本、模式、每文件 sha256）
和给用户看的 `README.txt`。`-Mode full` 要求 `dlssg_sm86\sr` 在库里存在（这两个文件不在 Git 里，
是本地文件）；`-Mode light` 会显式跳过并打印提示。

说明：`配置\鸣潮\Engine.ini` 只有在档案的 `config_source` 指向它（或游戏名/id 与目录同名）时才会被用上；
手动添加的游戏可以在「画质」里用 `05-自定义` 指到包内这份文件。

## 界面

主窗口一行一个游戏：`[启动] 名称 [帧生成] [画质] 状态 引擎 … [恢复原状] [移除]`，
底栏是 `[扫描游戏] 添加游戏 日志 关于`。

- **改名**：双击列表里的游戏标题进入编辑，输入框会全选原名，回车保存；Esc 放弃，
  点别处也算保存。只改偏好里的 `name`——id、目录、部署记录都不动，所以引擎在跑或有未完成
  部署时也能改；名字只影响 `<根>\配置\<游戏名>\Engine.ini` 这条兜底配置源（显式写过
  `config_source` 的档案不受影响），空白名字或超过 60 字会被拒绝并保留原名。
- **切换显卡**（底栏「添加游戏」后面）：改系统里显卡的显示名称，只改注册表的 `DeviceDesc`，
  不动驱动、不改变真实能力。窗口里设备只读显示，目标名可编辑；**只支持单张 NVIDIA 显卡**
  （核显与 N 卡的 HD 音频功能都忽略，多张直接报错不猜），默认填 `NVIDIA GeForce RTX 5070`，
  之后用上次成功切换的名字。状态备份与 PS 工具同路径同字段
  （`%ProgramData%\Toggle-NvidiaDeviceName\state.json`），两个工具可以互相还原；
  没有备份时按驱动 `DriverDesc` 兜底还原。改名与还原都要重启才生效：**本次运行里改成功后**
  窗口里出现红字「立即重启电脑」（切换和还原都出现，重启电脑或重开启动器后消失）；
  如果随后又改回**本次运行开始时的名字**（净改动为零），提示与按钮直接消失，不用重启。
  点它先二次确认，再 `shutdown /r /t 10`；引擎正在跑（部署/恢复/游戏）时改名与重启都置灰。

- **扫描游戏**：只扫 Steam / GOG / Epic 的平台记录与注册表（不翻全盘）。有证据的
  （UE 打包本体或同目录自带 DLSS-G 组件、或鸣潮/异环的内置模板）直接加进列表，
  其余进「扫描结果」等确认；每行一个「选择渲染程序」入口。
  Steam 游戏一律写成 **Steam 启动**（`steam.exe -applaunch <AppID>`，AppID 只从
  `appmanifest_*.acf` 读）；找不到 `Steam.exe` 时进「扫描结果」说明原因，不生成会绕开 Steam 的档案。
  Epic 游戏写成 **Epic 启动**（`com.epicgames.launcher://apps/<AppName>?action=launch`，
  AppName 只从 `LauncherInstalled.dat` 读）；GOG 保持 Direct——DRM-free，直接起 EXE 就是正常方式。
  鸣潮这类"注册表只登记启动器、游戏本体装在别的盘"的情况（真机：启动器在
  `C:\Program Files\Wuthering Waves`，游戏在 `C:\Games\Wuthering Waves Game`）另有一层
  **浅层兜底**：在固定磁盘前 3 层里按模板的文件夹名找，且必须 `<目录>\<渲染 EXE>` 真的存在
  才算，找到多个只报歧义；实测 3 个固定磁盘只读 317 个目录、16 毫秒左右，有目录上限并跳过系统目录。
- **添加游戏**：手动选目录 → 扫描候选 → 确认渲染程序。目录在平台库里时自动改成平台启动：
  Steam（`<库>\steamapps\common\<installdir>` 且有匹配清单，`Steam.exe` 取注册表或库根）或
  Epic（`LauncherInstalled.dat` 里目录对得上，写 AppName）；两个平台都对不上才保持 Direct。
  档案写着 Steam/Epic 却缺 AppID、AppName 或 `Steam.exe` 时，引擎**直接拒绝规划**，不会退回
  "直接起渲染 EXE"——那等于绕开平台，Steamworks 游戏（影之刃零）还会自己要求 Steam 重启，
  部署会在真正在跑的进程出现之前被恢复，日志看着"部署完成"，游戏却没注入。
- **帧生成**：12 个组件键（常用 5 + 高级 7）+ 自定义 INI + 代理文件（6 选一，
  异环建议 `winmm.dll`，有遗留部署时代理整块灰掉）。
- **画质**：引擎版本（可重新识别 / 手动校准）+ 4 档画质 + `05-自定义`；
  只在画质视图保存时才改配置源，配置源不存在会直接拒绝保存。
- 启动/恢复都交给提权后的引擎子进程；恢复**不显示进度**，结束后弹结果卡片。
- **会话**：Steam、Epic 和通用 Direct 启动最多等 120 秒。等待过程中可点「中止并恢复」。
  游戏运行后可用「结束并恢复」，它仅结束路径核验通过的渲染程序和该游戏独立的官方启动器。
  鸣潮保留热重启窗口，异环保留首次退出策略。自动恢复允许官方启动器继续运行。
- **托盘**：左键单击图标 = 打开窗口，右键 = 菜单（显示窗口 / 打开日志目录 / 退出）。
  窗口的关闭按钮只是**收进托盘**（进程留着，左键能再打开）；托盘菜单的「退出」是**直接结束进程**
  （界面没有未落盘的状态，偏好改完即写）。启动游戏交接后窗口自动收进托盘，托盘悬停提示当前状态。
- **单实例 / 再次运行程序**：界面拿着 `Local\DlssgLauncherUi` 锁。再次双击 `DlssgLauncher.exe`
  （无论窗口是在后台、最小化、还是收进了托盘）都会把已有窗口**还原并放到最前**，新进程自己退出；
  不会出现"再点一次像没反应"或开出第二个窗口。实现见 `launcher-win/src/activate.rs`：一个命名事件
  `Local\DlssgLauncherUiActivate` + `AllowSetForegroundWindow` 借前台权限，界面收到信号后走
  **和托盘「显示窗口」同一条路径**（清掉交接后隐藏、`Visible(true)`、`Minimized(false)`、`Focus`），
  另有一次 `EnumWindows` 找窗口 `ShowWindow`/`SetForegroundWindow` 的兜底。
- **中文字体**：用系统 `msyh.ttc`（18.8 MB），先只读**内存映射**再交给 egui（`FontData::from_blob`），
  不整块读进堆；映射不可用（字体缺失、无权限、空文件）时回退 `fs::read`。专用内存因此少约 18.9 MB，
  见 `docs/M4-REPORT.md` §6，实现见 `launcher-win/src/fontmap.rs`。

## 调试与验收入口

```powershell
# 引擎：只检查 / 真跑 / 恢复
DlssgLauncher.exe --engine check --report out.json
DlssgLauncher.exe --engine run --game <id> --root <运行根> --report out.json
DlssgLauncher.exe --engine restore --game <id> --root <运行根> --report out.json

# 显卡名称：只读状态 / 切换 / 还原 / 清旧备份（写操作必须提权；界面自己会拉起提权子进程）
DlssgLauncher.exe --gpu status  --report out.json
DlssgLauncher.exe --gpu apply   --name "NVIDIA GeForce RTX 5070" --report out.json
DlssgLauncher.exe --gpu restore --report out.json
DlssgLauncher.exe --gpu reset-state --report out.json

# 界面自检（开窗 hold 毫秒后自动关闭，写报告）
DlssgLauncher.exe --ui-selftest 2200 out.json

# 界面预览（不连引擎，只摆出某个界面，配合 tools/shot-ui.ps1 截图）
DlssgLauncher.exe --ui-preview restore-ok|restore-fail|framegen|quality|scan
```

真机探针（`--ignored`，默认不跑）：

```powershell
cd rust
# 引擎识别：需要 DLSSG_UE_PROBE=<游戏目录>、DLSSG_UE_PROBE_EXE=<渲染 EXE>
cargo test -p launcher-core --lib unreal -- --ignored --nocapture probe_real_game
# 平台扫描：读本机 Steam/GOG/Epic 记录与注册表（内置游戏带浅层兜底），打印可添加/待确认清单
cargo test -p launcher --bin DlssgLauncher scan -- --ignored --nocapture probe_real_scan
# 显卡名称：只读枚举本机 NVIDIA 显卡、状态备份与窗口初始按钮，不写注册表
cargo test -p launcher gpu -- --ignored --nocapture probe_real_gpu
```

## 与现有实现的关系

- 现有 PowerShell 实现（`启动脚本/`）**未改动**，仍是当前可用的唯一实现。
- 本目录所有脚本都只读现有源码；写操作只发生在 `rust/` 内与 `%TEMP%`。
- 计划与任务清单见 [plan/plan.md](plan/plan.md)（源：`plan/plan.json`）。

## 常用命令

```powershell
# 重新生成样本（管理员身份可补真实软链接样本）
powershell -NoProfile -ExecutionPolicy Bypass -File rust\tools\gen-fixtures.ps1

# 刷新函数级清单
powershell -NoProfile -ExecutionPolicy Bypass -File rust\tools\gen-parity-inventory.ps1

# 一键检查（fmt + clippy + test）
powershell -NoProfile -ExecutionPolicy Bypass -File rust\tools\check.ps1

# 实测 spike
cd rust\spike; cargo build --release
powershell -NoProfile -ExecutionPolicy Bypass -File rust\tools\measure-spike.ps1 -HoldMs 4000
```
