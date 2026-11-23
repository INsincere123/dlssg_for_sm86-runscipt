# rust/ — Rust 单 EXE 启动器（M1 已完成）

M1（`launcher-core`，15 步）已完成并通过 184 + 2 个测试；下一步 M2 见 [plan/plan.md](plan/plan.md)（源：`plan/plan.json`）。

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
| `spike/` | egui + glow 的最小工程，只用于测指标，不进产品 |

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
