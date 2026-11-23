# rust/ — Rust 单 EXE 启动器（M1–M6、M8–M9 已完成；M7 真机验收待做）

界面 + 引擎都在一个 `DlssgLauncher.exe` 里：不带参数是单窗口界面，带 `--engine` 是引擎。
计划与任务清单见 [plan/plan.md](plan/plan.md)（源：`plan/plan.json`）。

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

## 界面

主窗口一行一个游戏：`[启动] 名称 [帧生成] [画质] 状态 引擎 … [恢复原状] [移除]`，
底栏是 `[扫描游戏] 添加游戏 日志 关于`。

- **扫描游戏**：只扫 Steam / GOG / Epic 的平台记录与注册表（不翻全盘）。有证据的
  （UE 打包本体或同目录自带 DLSS-G 组件、或鸣潮/异环的内置模板）直接加进列表，
  其余进「扫描结果」等确认；每行一个「选择渲染程序」入口。
- **帧生成**：12 个组件键（常用 5 + 高级 7）+ 自定义 INI + 代理文件（6 选一，
  异环建议 `winmm.dll`，有遗留部署时代理整块灰掉）。
- **画质**：引擎版本（可重新识别 / 手动校准）+ 4 档画质 + `05-自定义`；
  只在画质视图保存时才改配置源，配置源不存在会直接拒绝保存。
- 启动/恢复都交给提权后的引擎子进程；恢复**不显示进度**，结束后弹结果卡片。

## 调试与验收入口

```powershell
# 引擎：只检查 / 真跑 / 恢复
DlssgLauncher.exe --engine check --report out.json
DlssgLauncher.exe --engine run --game <id> --root <运行根> --report out.json
DlssgLauncher.exe --engine restore --game <id> --root <运行根> --report out.json

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
# 平台扫描：读本机 Steam/GOG/Epic 记录与注册表，打印可添加/待确认清单
cargo test -p launcher --bin DlssgLauncher scan -- --ignored --nocapture probe_real_scan
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
