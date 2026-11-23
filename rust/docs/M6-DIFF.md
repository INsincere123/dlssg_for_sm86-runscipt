# M6 差分报告（PS ↔ Rust）

生成方式：`rust/tools/diff-parity.ps1`（2026-11-23 运行通过）  
链路：**PowerShell 现算 → 入库样本 ← Rust 断言**。样本最初由 `gen-fixtures.ps1` 生成；
本脚本在 M6 阶段用**当前** PS 实现重算一遍，确认样本没有随 PS 改动而失真。

## 1. 可自动比对的部分（差异 0）

| 项 | 数量 | 结果 |
|---|---|---|
| 诊断 INI（3 款 × 4 组参数） | 12 | 逐字节一致 |
| 诊断识别边界（`Accepted` 布尔） | 9 | 全部一致 |
| `启动配置.ini` 写回（7 种编码） | 7 | 逐字节一致（`tests/step05_config_writeback.rs`） |
| 路径归一化（13 条） | 13 | 完全一致（`tests/step04_config_paths.rs`） |
| journal 记录（真实 `steam-4356430`） | 1 | 字段级可读（见 §2 的 D-1） |
| `.state` 双根读取 | 1 | 命中 `启动脚本\.state\steam-4356430\active.json` |

## 2. 无法逐字节比对的部分与原因

| 项 | 差异 | 编号 |
|---|---|---|
| journal 文件字节 | 字段名/顺序一致，但 JSON 缩进与换行由各自序列化器决定；按字段比较 | D-1 |
| `dlssg_sm86.ini` 内容 | PS 把文本**硬编码在三款 Run-*.ps1 里**（每款不同，`MaxGeneratedFrames=5` 等），没有参数化渲染器；新程序用 12 键模型统一生成，默认值 `3/1/1` | D-2 |
| 日志文件命名/行格式 | 沿用 `<stem>_yyyyMMdd_HHmmss_fff_pid.log`，但行内容由新程序自己写 | D-4 |
| 偏好目录名 | `DlssgSteamLauncher` → `DlssgLauncher`，旧目录只读兼容 | D-3 |
| `package-manifest.json` | 新增 `Package` 字段（base/sr 分包），其余字段与 PS 一致 | D-10 |
| 恢复后残留检查 | PS 在删记录前内联调用；core 抽成 `assert_resources_removed` 由调用方执行 | D-9 |
| 引擎自动识别 | PS 用版本信息/PE 标记识别 UE4/UE5；新程序改为**用户声明** `EngineHost` | 决策 3.6 |

## 3. 本轮差分暴露并修复的缺陷

真实记录 `启动脚本\.state\steam-4356430\active.json` 的 `Phase` 是 `retained`，
而 `Journal.Phase` 只认 `deploying`/`recovering` → **升级后旧记录读不出来**。
三款 Run 脚本在 `-KeepLinks` 后都会写 `retained`。已补 `Phase::Retained` 并加用例。

## 4. 复跑方法

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File "D:\Games\dlssg_for_sm86\rust\tools\diff-parity.ps1"
powershell -NoProfile -ExecutionPolicy Bypass -File "D:\Games\dlssg_for_sm86\rust\tools\check.ps1"
```

任何一处不一致，脚本会列明细并以非零退出。

## 5. 未覆盖

- 真机行为（M6-03/04/05）：游戏内是否真正生效、热重启窗口、Steam 120 s 超时——都需要实际启动游戏。
- 恢复流程的**写操作**差分：本轮只读真实记录，未执行恢复（会改动 NBA 2K27 安装目录）。
