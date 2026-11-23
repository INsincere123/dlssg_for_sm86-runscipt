# PARITY 等价报告（Phase 1 收尾）

- 日期：2026-11-22
- 范围：Phase 1 的 15 步（`launcher-core`），不含 M2 及之后的引擎/平台/界面
- 判据：[PARITY.md](PARITY.md) 的 14 条兼容契约 + 8 条新增规则 + 8 条有意偏离
- 样本：[../fixtures/README.md](../fixtures/README.md)（51 个文件）

## 1. 总览

| 项 | 结果 |
|---|---|
| 步骤 | **15 / 15 全部实现** |
| `cargo test --workspace` | **306 通过 / 0 失败**（M5 后统计；M6-01/02 又加 5 个） |
| 真实软链接用例（`#[ignore]`） | **2 通过 / 0 失败**（2026-11-22 提权终端复测，无跳过、无 1314） |
| `cargo clippy --all-targets -- -D warnings` | 无告警 |
| `cargo fmt --all -- --check` | 无差异 |
| 一键检查 | `rust\tools\check.ps1`（fmt + clippy + test，可选 `-IncludeIgnored`） |
| 关卡 | A ✅ · B ✅ · C ✅（含真实软链接复测）· D ✅ · E = 本报告 |
| 与现有实现的字节级等价 | 诊断 INI 12 份、诊断模板 8 份、`启动配置.ini` 写回 7 种编码 |
| 字段级等价 | journal schema v1（5 个阶段快照） |
| 未验证 | 真实软链接创建/拆除、UAC 提权、组件对 12 键的实际接受度 |

## 2. 逐项等价证据

| 契约 | 等价方式 | 证据 |
|---|---|---|
| C1 journal schema v1 | 字段级（D-1） | 5 个阶段快照解析 + 往返相等；`null`/空串区分；版本 2 拒绝 |
| C2 互斥名与 identity | **逐字节** | identity = `UPPERHEX(SHA256(UTF8(normalized(render))))`，与 PS 公式逐字符一致 |
| C3 恢复顺序 | 行为等价 | 7 类顺序 + 同类逆序，用 6 条混合操作断言顺序 |
| C4 依赖保留 | 行为等价 | 3 条依赖关系各有用例；冲突时记录保留 |
| C5 归档命名 | **逐字节** | `recovered/<Session>-Engine.ini`，会话必须是 32 位十六进制；`prepare → restore` 往返用例覆盖 |
| C6 资源/代理/`_storage_` | 行为等价 | 6 代理映射、5 目录、代理链接部署/复用、第三方代理不覆盖、哈希认领（Owned/Foreign/Absent）、收尾残留检查；**junction 不被当成自己的链接**（恢复与拆除两侧各一个用例） |
| C7 诊断 INI | **逐字节** | 3 款 × 4 组参数 = 12 份样本全等；9 条识别边界与 PS 结果一致；部署侧：临时文件拒绝覆盖、`diagnostic` 记录用文本哈希、`dlssg_sm86.ini` 链接复用、被篡改则恢复时保留 |
| C8 runtime 副本 | **逐字节** | `engine_config::runtime_copy_bytes` = marker + CRLF + 源文本、UTF-8 无 BOM；四个 marker 常量逐字对齐；副本只读；恢复时先清属性再删（等价 `Remove-Item -Force`） |
| C9 三条会话策略 | 行为等价 | 热重启（含重启一次后结束）、首次退出、等全部目标；120 秒超时 / 3 秒关闭宽限 |
| C10 `启动配置.ini` | **逐字节** | 7 种编码读→写回保留编码与 BOM；路径语法 14 条用例 |
| C11 偏好目录 `<16hex>` | **纯函数 + 实测向量** | `prefs::path_key` 用 PS 实测向量锁定：`D:\Games\dlssg_for_sm86` → `0832029A5839CB2D`；新旧数据目录名都支持（D-3） |
| C12 日志 | 行为等价 | 目录、文件名规则、24 小时清理（跳过链接、失败忽略）、组件日志增量游标 |
| C13 恢复拒绝条件 | 行为等价 | 进程检查由调用方注入，`automatic` 与显式恢复分宽严 |
| C14 便携清单 | 未实现（M5） | — |

## 3. 有意偏离（与 PARITY `D-n` 对应）

| ID | 偏离 | 现状 |
|---|---|---|
| D-1 | journal 只保证字段级，不复刻 `ConvertTo-Json` 缩进 | 已实现，样本保留 PS 原始形态作参考 |
| D-2 | 运行时配置 12 键、默认 `3/1/1`（旧模板 8 键、`5/2/2`） | 已实现；12 键文本由 `Settings::render` 固定 |
| D-3 | 偏好目录改名 `DlssgLauncher` | 未实现（偏好层属 M4） |
| D-4 | 删除 UE 自动识别 | 在 M0 计划里删除，Phase 1 无相关代码 |
| D-5 | `sr` 资源按需 | 已实现（`required_files(false/true)` 5/7 项 + `Settings::requires_super_resolution_resources`） |
| D-6 | 单会话 | 未实现（引擎层属 M2；互斥名已就绪） |
| D-7 | 便携包形态 | 未实现（M5） |
| D-8 | 旧诊断文本按旧模板识别 | 已实现（`DiagnosticAcceptance`：模板重生成 + 旧文本逐字匹配） |

## 4. 与 PS 的真实差异（不只写在纸上的）

1. **`StartupTimedOut`/`LauncherClosedBeforeGame`**：PS 返回 `$null` 让调用方走恢复；
   Rust 返回具名结果，语义一致但可区分原因（日志更明确）。
2. **`WaitAllTargets`**：PS 在 Steam/Direct 里按完整 EXE 路径判定；
   core 只拿到进程名集合，路径判定留给 M3 的平台层（接口已按 `ProcessEntry.executable` 预留）。
3. **收尾残留检查（D-9）**：PS 在删记录前调 `Assert-DeploymentResourcesRemoved`；core 的 `restore` 拿不到部署上下文，因此把它做成 `resources::assert_resources_removed(deployment, runtime, backup, ops)`，由调用方在删记录前调用。\n4. **`restore` 的 `Reused` 导入**：PS 用 `Assert-DeploymentLinkSpec` 对照部署上下文；
   core 要求调用方显式传入允许的链接规格，空切片即跳过。安全性更强，但调用方必须传对。

## 5. 未验证 / 遗留

| # | 项 | 影响 | 补法 |
|---|---|---|---|
| 1 | ~~真实软链接的创建、校验、拆除~~ | **已验证（2026-11-22）**：提权终端跑 `check.ps1 -IncludeIgnored`，`real_symlinks` 2/2 通过（目录链接只删链接、目标保留；文件链接被替换后先归档再恢复原件）。非提权环境仍会失败而不是假通过。 | 已完成 |
| 2 | UAC 提权隐藏子进程 | M2 才用；M0 spike 已写出代码路径 | 手工确认一次 UAC |
| 3 | 组件对 12 键文本的实际接受度 | D-2 的正确性 | M6 真机 + 组件日志 |
| 4 | `sr` 缺失时组件行为 | D-5 是否成立 | M6 真机 |
| 5 | Steam/Direct 的完整路径判定 | M3 | M3 实现时补 |

## 6. 结论

Phase 1 **已完成**：15 步全部实现，**184 个常规测试 + 2 个真实软链接测试全部通过**，
覆盖 14 条兼容契约里能自动验证的部分；诊断 INI 与配置写回达到逐字节等价。

关卡 A–E 全过。真实软链接不再是缺口：2026-11-22 在提权终端用
`rust\tools\check.ps1 -IncludeIgnored` 复测，`real_symlinks` 2/2 通过——
目录链接只删链接本身、目标原样保留；文件链接被游戏替换成普通文件时先归档再恢复原件。

进入 M2 前仍属"未验证"的只剩真机相关项：UAC 提权隐藏子进程、组件对 12 键文本的实际接受度、
`sr` 缺失时的组件行为、Steam/Direct 的完整路径判定（M3 补）——它们都在 M2/M3/M6 的计划里。

