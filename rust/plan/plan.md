# 开发计划（生成物，源：plan.json）

- 生成时间（UTC）：2026-11-22T16:34:40.4889663Z
- 版本：2.0　受众：ai-agent
- 说明：plan.json 是唯一事实来源；plan.md 由 rust/tools/gen-plan.ps1 生成，不要手改 md。

## 入口

| 键 | 路径 |
|---|---|
| `plan` | `rust/plan/plan.json` |
| `generated` | `rust/plan/plan.md` |
| `contracts` | `rust/docs/PARITY.md` |
| `equivalence_report` | `rust/docs/PARITY-REPORT.md` |
| `progress` | `rust/docs/phase1-progress.md` |
| `m0_report` | `rust/docs/M0-REPORT.md` |
| `fixtures` | `rust/fixtures/README.md` |

## 环境

| 项 | 值 |
|---|---|
| os | Windows 11 26300 x64 |
| shell | pwsh 7.6.6；PS 5.1 用于 .ps1 工具 |
| elevated | True |
| symlink_privilege | SeCreateSymbolicLinkPrivilege 可用（High 完整性，2026-11-22 验证） |
| toolchain | rustc/cargo 1.99.1 x86_64-pc-windows-msvc |
| gpu | RTX 3080 Ti Laptop / 4K 150% |

## 命令

| 名称 | 命令 |
|---|---|
| `check` | `powershell -NoProfile -ExecutionPolicy Bypass -File "D:\Games\dlssg_for_sm86\rust\tools\check.ps1"` |
| `check_all` | `powershell -NoProfile -ExecutionPolicy Bypass -File "D:\Games\dlssg_for_sm86\rust\tools\check.ps1" -IncludeIgnored` |
| `fixtures` | `powershell -NoProfile -ExecutionPolicy Bypass -File "D:\Games\dlssg_for_sm86\rust\tools\gen-fixtures.ps1"` |
| `inventory` | `powershell -NoProfile -ExecutionPolicy Bypass -File "D:\Games\dlssg_for_sm86\rust\tools\gen-parity-inventory.ps1"` |
| `plan` | `powershell -NoProfile -ExecutionPolicy Bypass -File "D:\Games\dlssg_for_sm86\rust\tools\gen-plan.ps1"` |
| `build` | `cd rust; cargo build --release` |
| `test` | `cd rust; cargo test --workspace` |

## 规则（违反即为缺陷）

| ID | 规则 | 强制方式 |
|---|---|---|
| R1 | 只写 rust/ 与 rust/plan/；启动脚本/ 视为只读参照，除非任务显式要求改动 | git status 检查路径 |
| R2 | 每个任务收尾必须跑 check，且 fmt/clippy/test 全绿才允许标记 done | commands.check |
| R3 | 禁止手改 rust/fixtures/**；样本只能由 gen-fixtures.ps1 生成 | fixtures/manifest.json 的 Sources 哈希 |
| R4 | 行为或契约变化必须先改 rust/docs/PARITY.md（含新增 D-n）再改代码 | review diff |
| R5 | 不提交、不推送，除非用户明示 | 用户指令 |
| R6 | rust/fixtures/** 的换行由 .gitattributes 的 -text 保护，不得移除该规则 | git ls-files --eol |
| R7 | 恢复顺序、互斥名、identity 公式、recovered 命名不得改动 | tests/step07_journal_store.rs、tests/step10_11_restore.rs |
| R8 | 鸣潮热重启 / 异环首次退出 / Steam-Direct 等全部目标，三条策略不得统一 | src/session.rs 单测 |
| R9 | 真实软链接用例必须真实建链接；无权限时失败而不是跳过 | DLSSG_REQUIRE_SYMLINK_TESTS=1 |
| R10 | 恢复失败时保留记录与现场；只有全部步骤成功且残留检查通过才能删记录 | tests/step10_11_restore.rs、resources::assert_resources_removed |
| R11 | core 不得引用 Win32 或 UI 依赖；平台能力只能经 ops 的 4 个 trait 注入 | cargo tree + review |
| R12 | 任何与现有 PS 实现的差异都要有 D-n 编号并写明验证方式 | PARITY.md §4 |

## 关卡

| 关卡 | 状态 | 范围 | 证据 |
|---|---|---|---|
| A | pass | config 读取/路径/写回 | 59 测试；PARITY-REPORT §2 |
| B | pass | journal 结构/落盘/identity | 5 个阶段快照字段级一致 |
| C | pass | resources + restore + 真实链接 | real_symlinks 2/2（提权，2026-11-22） |
| D | pass | settings 12 键 + 诊断模板 | 12 份诊断样本逐字节一致；9 条识别边界同 PS |
| E | pass | session + log + 差分报告 | PARITY-REPORT.md；check 全绿 |
| F | pass | M2 引擎壳：launcher-win 真实现 + CLI + 提权 + IPC + 单会话 + 复用启动器 | 208 测试通过（M2 新增 14 个）；check.ps1 全绿；未验证：从非提权父进程触发 UAC 弹窗（本会话已是高完整性，弹窗路径测得的是 classify_shell_error(1223) 单测） |
| G | pass | M3 入口层：Steam 库/清单/AppID、Direct 身份与相对 EXE、发现扫描、PE 代理推荐、画质档位 | 251 测试通过（M3 新增 31 个）；Steam 与 Direct 语义逐条对照 PS 源码与测试；注册表记录读的是本机真实键值（无 panic 且有存在性断言）；未验证：真实游戏的启动分发（引擎流程仍为壳）、Epic 记录在本机无样本 |
| H | pass | M4 界面：主窗/设置页/自定义 INI/添加游戏/日志与关于/性能与 DPI | 274 测试通过；docs/M4-REPORT.md 记录实测（生产 WS 84.3 MB、自检 87.0 MB、首帧 128–161 ms、ppp=1.5、单实例 64 ms 退出）；未验证：手工交互、多显示器与 100%/200% 缩放 |

## 任务（done 33 / todo 9）

### M1

| ID | 任务 | 状态 | 依赖 | 验收 | 产物 |
|---|---|---|---|---|---|
| M1-01 | workspace + 错误类型 + 4 个能力 trait + 样本读取器 | done | — | cargo test -p launcher-core 通过；clippy 无告警 | rust/Cargo.toml、src/ops.rs、src/error.rs、src/fixtures.rs |
| M1-02 | path：GetFullPath/TrimEnd/ToLowerInvariant 等价 + SHA256 大写十六进制 | done | M1-01 | fixtures/paths/normalized-path.json 13 条全等 | src/path.rs |
| M1-03 | config 读取：编码/BOM 顺序/中文键+别名/行号报错 | done | M1-02 | 7 种编码样本解码为同一文本 | src/config.rs、fixtures/config/encodings |
| M1-04 | config 路径语法：env/~/file URI/\\?\/盘符相对拒绝/非法字符 | done | M1-03 | tests/step04_config_paths.rs 14 条 | src/config.rs |
| M1-05 | config 写回：保留编码与 BOM + Local 互斥 + 5s 等待 | done | M1-04 | 7 种编码字节级保留且只改目标行 | src/config.rs、tests/step05_config_writeback.rs |
| M1-06 | journal：schema v1 结构、null 与空串、Operations/Reused 顺序 | done | M1-05 | 5 个阶段快照解析 + 往返相等 | src/journal.rs |
| M1-07 | journal 落盘与会话：原子写、父目录重解析点拒绝、identity、状态机 | done | M1-06 | tests/step07_journal_store.rs 10 条 | src/journal_store.rs |
| M1-08 | resources：5 目录 + 6 代理 + sr 分层断言 + _storage_ 认领 + 残留检查 | done | M1-07 | tests/step08_09_resources.rs 16 条 | src/resources.rs |
| M1-09 | resources 部署侧：代理链接、Engine.ini 只读副本/备份/链接、诊断 INI | done | M1-08 | tests/step08_engine_config.rs 8 条 + step08_diagnostic.rs 7 条 | src/engine_config.rs、src/diagnostic.rs |
| M1-10 | restore：7 步顺序 + 依赖保留 + 冲突保留 | done | M1-09 | tests/step10_11_restore.rs 顺序与依赖断言 | src/restore.rs |
| M1-11 | restore：归档 + engine_only/preview/automatic + Reused 导入 | done | M1-10 | 归档命名与冲突保留用例；真实链接 2/2 | src/restore.rs、tests/real_symlinks.rs |
| M1-12 | settings：12 键模型、校验、渲染、标记行、导入 | done | M1-11 | 发行 INI 导入 11 键；渲染往返相等 | src/settings.rs、tests/step12_13_settings.rs |
| M1-13 | settings：诊断模板生成与识别、旧键提示 | done | M1-12 | 12 份诊断样本逐字节；9 条识别边界同 PS | src/settings.rs |
| M1-14 | session：三策略状态机 + 启动超时/关闭宽限 + 复用已运行启动器 | done | M1-13 | tests/step14_15_session_log.rs 6 条 + 单测 | src/session.rs |
| M1-15 | log + prefs：日志命名/24h 清理/组件游标、偏好目录 <16hex> | done | M1-14 | 单测 + PowerShell 实测向量 0832029A5839CB2D | src/log.rs、src/prefs.rs |

### M2

| ID | 任务 | 状态 | 依赖 | 验收 | 产物 |
|---|---|---|---|---|---|
| M2-01 | launcher-win crate：SymlinkOps/MutexOps/ProcessProbe/Clock 的 Windows 真实现 | done | M1-15 | 真实链接与互斥单测通过；目录符号链接用 RemoveDirectory、按重解析标签区分 junction | rust/crates/launcher-win/ |
| M2-02 | CLI：--engine run\|restore\|check，windows 子系统 + CREATE_NO_WINDOW | done | M2-01 | 引擎进程 GetConsoleWindow()==0 | rust/crates/launcher/ |
| M2-03 | 提权：ShellExecuteEx(runas) + 取消 UAC 保持 GUI 可用 + 返回码回传 | done | M2-02 | 手工一次 UAC 确认；取消时 GUI 不退出 | launcher-win |
| M2-04 | IPC：命名管道（显式 DACL）+ status.json 兜底 | done | M2-03 | 管道不可用时读 status.json 得到同一阶段 | launcher-win、core::session |
| M2-05 | 单会话 Global\DlssgSession + 第二款游戏拒绝 | done | M2-04 | 并发启动第二个游戏返回明确错误 | launcher-win |
| M2-06 | 复用已运行的官方启动器（完整 EXE 路径） | done | M2-05 | 启动器已运行时不再拉起、直接进入等待 | core::session、launcher-win |

### M3

| ID | 任务 | 状态 | 依赖 | 验收 | 产物 |
|---|---|---|---|---|---|
| M3-01 | Steam：libraryfolders.vdf / appmanifest / AppID 匹配 / -applaunch | done | M2-06 | 对齐 SteamUE5.Tests 语义；零/多匹配报原因不猜 | core::steam |
| M3-02 | Direct：相对 EXE 校验、工作目录、direct-<GUID> 身份 | done | M3-01 | 对齐 DirectLauncher.Tests | core::direct |
| M3-03 | 游戏发现：Steam/Epic/GOG 记录 + 候选扫描、打分、12 层/12000 目录上限、可取消 | done | M3-02 | 不按盘符全盘扫描；超限可取消 | core::discovery |
| M3-04 | 代理推荐：PE 导入表扫描，只推荐存在的代理名 | done | M3-03 | 伪造 PE 样本的导入表命中 | launcher-win |
| M3-05 | quality：EngineHost(none/UE4/UE5) → 档位路径 | done | M3-04 | 纯函数单测覆盖 none 与 01-05 档位 | core::quality |

### M4

| ID | 任务 | 状态 | 依赖 | 验收 | 产物 |
|---|---|---|---|---|---|
| M4-01 | GUI 骨架：egui+glow、单实例、交接成功后退出 | done | M3-05 | 实测 2026-11-22 release：WS 83.9 MB（含微软雅黑）、首帧 165 ms、ppp=1.5；采样需在稳定后 ≥900 ms；第二个实例 64 ms 内退出且不开窗 | rust/crates/launcher |
| M4-02 | 主窗：游戏列表 + 状态胶囊（可启动/需恢复/会话中/需定位）+ 偏好读写 | done | M4-01 | 四种状态都能渲染；布局无重叠 | launcher/ui |
| M4-03 | 设置页：常用 5 项 + 高级 7 项 + 联动灰化 + 严格校验 | done | M4-02 | 12 键与 settings::validate 一致；XeSS 上限联动 | launcher/ui |
| M4-04 | 自定义 INI：导入、预览生成结果、恢复默认 | done | M4-03 | 导入未知键报提示但仍保留原文 | launcher/ui |
| M4-05 | 添加游戏：扫描 → 候选 → 确认 + EngineHost 三选一 | done | M4-04 | 唯一可信结果自动采用，异常才提示 | launcher/ui |
| M4-06 | 日志入口、关于（程序版本 + 组件基线）、通知 | done | M4-05 | 通知不抢焦点；日志入口打开 logs 目录 | launcher/ui |
| M4-07 | 性能与 DPI 验收：150% 缩放、中文渲染、内存/启动达标 | done | M4-06 | ppp=1.5 无模糊；WS 与启动时间写进报告 | docs/ |

### M5

| ID | 任务 | 状态 | 依赖 | 验收 | 产物 |
|---|---|---|---|---|---|
| M5-01 | 显卡名工具：CfgMgr32 枚举 PCI\VEN_10DE + DeviceDesc 读写 + ProgramData 状态备份 | todo | M4-07 | 隔离模拟注册表通过（对齐 NvidiaDeviceName.Tests） | launcher-win |
| M5-02 | 便携导出：基础包 + sr 配件、package-manifest FormatVersion=1 | todo | M5-01 | 文件集合与体积符合 PARITY 12.4 | core::export |
| M5-03 | 导出自检：逐文件 SHA256 + 结构校验 | todo | M5-02 | 篡改任一文件后自检失败 | core::export |

### M6

| ID | 任务 | 状态 | 依赖 | 验收 | 产物 |
|---|---|---|---|---|---|
| M6-01 | .state 双根读取与迁移（新根优先，旧 启动脚本\.state 只读兜底） | todo | M5-03 | 读取现有 steam-4356430 记录并恢复成功 | core::journal_store |
| M6-02 | 差分测试：PS 与 Rust 同输入 dry-run 比对 journal/生成文件/日志行 | todo | M6-01 | 差异为 0 或全部有 D-n 解释 | rust/tools/ |
| M6-03 | 真机：鸣潮（含 10s 热重启） | todo | M6-02 | 启动→退出→恢复完整；无残留 | logs/ |
| M6-04 | 真机：异环（首次退出即恢复） | todo | M6-03 | 同上，且策略未被统一 | logs/ |
| M6-05 | 真机：Steam 与直接 EXE（等全部目标 + 120s 超时） | todo | M6-04 | 超时后正常恢复 | logs/ |
| M6-06 | 文档同步与收尾报告（README/使用说明/配置参考/日志规范） | todo | M6-05 | 四份文档与实际行为一致 | docs/ |

