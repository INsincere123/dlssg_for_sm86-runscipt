# 黄金样本（rust/fixtures）

M0 冻结的字节级判据。**不要手工编辑**：全部由 `rust/tools/gen-fixtures.ps1` 调用现有 PowerShell 实现生成。

## 怎么再生

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File rust\tools\gen-fixtures.ps1
```

- 只在 `%TEMP%` 下建临时目录，跑完删除；不启动游戏、不写真实游戏目录、不碰仓库 `.state`。
- `-KeepTemporary` 保留临时目录用于人工核对。
- 每次生成都会重写 `manifest.json`，内含每个样本的 SHA256 与源文件哈希，可用来确认"样本对应当前源码"。

## 目录

| 路径 | 内容 | 对应契约 |
|---|---|---|
| `templates/<game>/0.4.2.ini` | 各入口 0.4.2 诊断模板原文 | C7 |
| `templates/<game>/0.4.0.ini` | 0.4.0 模板（更早的部署要靠它识别） | C7 / D-8 |
| `templates/wuthering/0.3.2.ini` | 鸣潮 0.3.2 模板 | C7 |
| `templates/wuthering/0.4.0-mode-off.ini` | 0.4.0 模板经 `Mode=On → Mode=Off` 派生（`Run-WutheringWaves-DLSSG.ps1:140`） | C7 |
| `templates/index.json`、`derived-index.json` | 模板来源与派生规则 | — |
| `diagnostic/<game>/<combo>.ini` | 3 款 × 4 组参数（`default` / `sr-m-int8` / `sr-l-fp16` / `fair`）的生成结果 | C7 |
| `diagnostic/recognition.json` | 识别器 8 条接受/拒绝边界 | C7 |
| `journal/01-new.json` … `07-no-original.json` | journal 各阶段快照 | C1 |
| `journal/05-recovered.txt` | 恢复完成后的不变量说明 | C1 / C3 |
| `journal/06-recovered-archive.json`、`.name.txt` | 被替换 `Engine.ini` 的归档内容与命名 | C5 |
| `paths/normalized-path.json` | 归一化表达式 + 13 条用例 + 生成时的 cwd 与 PS 版本 | C2 / C3 |
| `manifest.json` | 全部样本的 SHA256、源文件哈希、Git 提交、生成时间 | — |

## journal 快照的归一化

生成时把易变值替换成占位符，其余字节（含 PS `ConvertTo-Json` 的缩进形态）原样保留：

| 占位符 | 替换内容 |
|---|---|
| `<TEMPROOT>` | 本次生成的临时根目录（含 JSON 转义后的双反斜杠） |
| `<SESSION>` | `Session` 字段（32 位十六进制） |
| `<PID>` | `ScriptPid` |
| `<STARTED>` | `ScriptStarted`（ISO-8601 UTC） |

Rust 侧比对时按 §4 D-1 只要求**字段级等价**：字段名、类型、`null` 与空串的区分、`Operations[]` 顺序与 `State` 流转必须一致，缩进不复刻。

## 已知边界

1. **`journal/06` 的链接是模拟的**：当前会话没有 `SeCreateSymbolicLinkPrivilege`，样本用"记录为 `done` + 放置普通文件"走同一条归档分支（`DeploymentJournal.ps1:438-450`）。真实软链接的部署/拆除路径还没有字节样本，需以管理员身份重跑本生成器补上。
2. `05-recovered.txt` 与 `06-recovered-archive.name.txt` 是断言说明，不是 JSON 样本。
3. 样本不含任何真实游戏路径、用户名或本机目录。

## 与 PARITY 的关系

契约编号（C1–C14、N-1–N-8、D-1–D-8）见 [../docs/PARITY.md](../docs/PARITY.md)。
