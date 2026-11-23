# Phase 1 进度（M1）

- 更新：2026-11-22（**15 步全部实现；关卡 A–E 全过**，含提权终端下的真实软链接复测）
- 计划：[rust开发.md](../plan/plan.md) §5 Phase 1（15 步 / 5 关卡）
- 契约：[PARITY.md](PARITY.md)

## 步骤状态

| 步 | 内容 | 状态 | 证据 |
|---|---|---|---|
| 1 | workspace、错误类型、4 个 trait、样本读取器 | ✅ | `tests/step01_skeleton.rs`（7） |
| 2 | `path`：归一化 / 全路径 / SHA256 | ✅ | `src/path.rs`（11）＋ 13 条样本用例 |
| 3 | `config` 读取：编码 / BOM / 中文键 / 行号报错 | ✅ | `src/config.rs`（11）＋ 7 个编码样本 |
| 4 | `config` 路径语法 | ✅ | `tests/step04_config_paths.rs`（14） |
| 5 | `config` 写回：保留编码 + BOM + 互斥 | ✅ | `tests/step05_config_writeback.rs`（10） |
| 6 | `journal` 结构与序列化（schema v1 字段级） | ✅ | `src/journal.rs`（11） |
| 7 | `journal` 原子落盘 + parents 校验 + 状态机 + identity | ✅ | `src/journal_store.rs`（6）＋ `tests/step07_journal_store.rs`（10） |
| 8 | `resources` 规格与断言（5 目录 + 6 代理 + `sr` 分层 + `_storage_` 认领）＋代理链接部署＋诊断 INI 部署 | ✅ | `src/resources.rs`（3）、`src/diagnostic.rs`（1）＋ 两个集成文件（16 + 7） |
| 9 | `resources` 安装/复用/拆除（写进 journal）＋`Engine.ini` 只读副本/备份/链接＋收尾残留检查 | ✅ | `tests/step08_09_resources.rs`（14）＋ `tests/step08_engine_config.rs`（8） |
| 10 | `restore` 7 步顺序 + 依赖保留 + 冲突保留 | ✅ | `src/restore.rs`（1）＋ `tests/step10_11_restore.rs`（13） |
| 11 | `restore` 归档 + `engine_only`/`preview`/`automatic` + `Reused` 导入 | ✅ | 同上 |
| 12 | `settings` 12 键模型、校验、渲染、标记行、导入 | ✅ | `src/settings.rs`（9）＋ `tests/step12_13_settings.rs`（11） |
| 13 | `settings` 诊断模板生成与识别、旧键提示 | ✅ | 同上（12 份诊断样本逐字节比对） |
| 14 | `session` 状态机 + 三策略 + 启动器三阶段等待 + 复用已运行启动器 | ✅ | `src/session.rs`（11）＋ `tests/step14_15_session_log.rs`（6） |
| 15 | `log` 结构化日志 + 24h 轮转 + 组件日志游标 | ✅ | `src/log.rs`（8）＋ 同上 |

## 关卡 B（步 6–7）

| 条件 | 结果 |
|---|---|
| 5 个 journal 阶段快照逐字段解析（01/02/03/04/07） | 通过 |
| 字段级往返（parse → to_json → parse 相等） | 通过 |
| `null` 与空串区分（`OriginalHash`、`Target`/`Hash`） | 通过 |
| 版本 2 拒绝、未知字段容忍、会话标识校验 | 通过 |
| 原子落盘不残留 `.tmp`；记录路径是目录时拒绝 | 通过 |
| identity = `UPPERHEX(SHA256(UTF8(normalized(render))))` 与 PS 公式一致 | 通过 |
| `ScriptStarted` = 7 位小数 + `Z`（含闰日用例） | 通过 |
| 会话状态机 intent→done→restored、阶段切换、收尾删记录并释放两个互斥 | 通过 |
| 同游戏第二次会话被拒（`WaitOne(0)`） | 通过 |

## 关卡 A（步 1–5）

| 条件 | 结果 |
|---|---|
| `cargo test --workspace` | 59 通过 / 0 失败（关卡 A 时点） |
| `cargo clippy --workspace --all-targets -- -D warnings` | 无告警 |
| config 读取（7 种编码 → 同一文本） | 通过 |
| config 路径语法（env / `~` / URI / `\\?\` / 盘符相对 / 非法字符） | 通过 |
| config 写回（7 种编码字节级保留、只改目标行） | 通过 |
| trait 冻结 | 见下节，已冻结 |

## 已冻结的接口（关卡 A 之后不再修改）

```rust
// src/ops.rs —— core 唯一的平台能力入口
pub trait SymlinkOps {
    fn create(&self, link: &Path, target: &Path, target_is_dir: bool) -> Result<()>;
    fn inspect(&self, path: &Path) -> Result<Option<LinkInfo>>;
    fn remove(&self, path: &Path) -> Result<()>;
}

pub trait MutexOps {
    fn acquire(&self, name: &str, wait: Duration) -> Result<MutexHandle>;
    fn release(&self, handle: MutexHandle) -> Result<()>;
}

pub trait ProcessProbe {
    fn list(&self) -> Result<Vec<ProcessEntry>>;
    fn current(&self) -> Result<ProcessEntry>;
    fn started_at(&self, pid: u32) -> Result<SystemTime>;
}

pub trait Clock {
    fn now(&self) -> SystemTime;
}

pub struct Platform<'a> { symlink, mutex, process, clock }   // 单一注入点
pub struct MutexLease<'a>                                    // RAII 释放
```

`LinkKind` 区分 `SymbolicLink` / `Junction` / `OtherReparse`：恢复流程必须能拒绝 junction
（`DeploymentJournal.ps1:453` 只认 `SymbolicLink`）。

## 本阶段新增的样本

`rust/fixtures/config/encodings/`：同一份逻辑文本的 7 种编码（utf8、utf8-bom、utf16-le/be、
utf32-le/be、gbk）+ `index.json`（记录原始文本与文件清单）。

样本内容刻意包含：整行 `;` / `#` 注释、中文键、带引号的值、值里含 `#` 与 `=`、
以及一个**值为空**的键（写回替换分支的输入）。

`journal/*.json` 的占位符已改成**合法 JSON 值**（`"Session": "<SESSION>"`、
`"ScriptPid": 0`、`"ScriptStarted": "<STARTED>"`、路径里的 `<TEMPROOT>`），
因此测试可以直接解析样本。会话标识的校验放在 `Journal::validate`（使用前调用），
不放在 `from_json`。

## 当前总数

`cargo test --workspace`：**184 通过 / 0 失败**；`cargo clippy --all-targets -- -D warnings` 无告警；`cargo fmt --all -- --check` 无差异。一键检查：`rust\tools\check.ps1`（加 `-IncludeIgnored` 会跑真实链接用例）。

## 已知限制

1. `tests/step05_config_writeback.rs::symlink_is_refused_before_touching_anything`
   在无 `SeCreateSymbolicLinkPrivilege` 时**直接返回**（跳过），不会失败也不会伪造结果。
2. 写回使用 `fs::rename` 覆盖，未复刻 `File.Replace` 的 ACL/备用流语义；
   目标文件在写回前会检查只读属性（现有实现同样拒绝只读文件）。
3. `config::decode` 的 GBK 回退依赖 `encoding_rs`；非 UTF-8 且非 GBK 的文件会得到替换字符，
   与现有实现的"警告后继续"一致，不做静默失败。
4. `journal_store::assert_ordinary_parents` 的"父目录是重解析点"分支没有自动化用例
   （当前会话无法创建链接），与 `restore` 的真实链接样本一并留到关卡 C 补。

## 步 8–9 的实现要点

- 代理编号 `1..6` 严格映射，`0`/`7`/文件名一律拒绝；`from_name` 大小写不敏感。
- `assert_deployment` 按 PS 顺序检查：必需文件 → 其他 5 个代理名位置（指向项目的旧链接 = 停止，
  普通文件 = 记录后共存）→ 资源根必须是普通目录 → 5 个资源位置只能是"指向项目对应目录的链接"。
- `install_resources` 幂等：已存在的正确链接按"复用"登记，不重复创建也不重复记录；
  资源根第一次创建时记一条 `directory` 操作。
- `undo_resources` 只删自己的链接；别人占住的普通目录保留并报告；资源根只在空时删除。
- `claim_storage` 按 SHA256 与项目代理比对：一致 `Owned`、不一致 `Foreign`、不存在 `Absent`。
- 测试里"链接存在"由 `FakeSymlinkOps` 表达，"普通文件/目录占用"由真实临时目录表达，
  两者组合覆盖"链接被替换成普通目录"的分支，不需要管理员权限。

## 关卡 C（步 8–11）

| 条件 | 结果 |
|---|---|
| 必需资源按 `sr` 分层断言（5 / 7） | 通过 |
| 其他代理名：指向项目的旧链接停止、普通文件共存 | 通过 |
| 资源根 / 5 个资源位置冲突保留 | 通过 |
| 安装幂等（复用不重复记录）、演练零副作用 | 通过 |
| 拆除只删自己的链接、资源根仅在空时删 | 通过 |
| `_storage_` 按哈希认领（Owned / Foreign / Absent） | 通过 |
| 7 步顺序 + 同类逆序 | 通过 |
| 依赖保留（engineLink → backup/runtime、backup → runtime、link → diagnostic） | 通过 |
| 冲突只记录不中断（最终 `Recovery incomplete; journal retained`） | 通过 |
| 被替换 `Engine.ini` 归档、归档已存在时两边都保留 | 通过 |
| `preview` 只列计划、`engine_only` 只碰画面配置、`automatic` 只查游戏 | 通过 |
| `Reused` 导入按允许规格校验 | 通过 |
| **隔离目录 部署 → 中断 → 恢复 端到端** | 通过（`deploy_then_interrupt_then_restore_round_trip`） |
| **真实软链接样本/验证** | ✅ 2026-11-22 提权终端 `check.ps1 -IncludeIgnored`：`real_symlinks` 2/2 通过 |

> 关卡 C 的唯一缺口是"真实软链接样本"。功能路径已被端到端用例覆盖，但**链接的真实创建/拆除**
> 只在 M0 spike 之外没有实机验证。补法：以管理员身份重跑 `rust/tools/gen-fixtures.ps1`
> 并在提权终端跑一次 `cargo test -- --ignored`（真实链接用例标 `#[ignore]`，见下一步）。

## 步 10–11 的实现要点

- 顺序用 `Journal::restore_order_indices()`（下标版）：恢复时要边执行边改 `State`。
- 依赖检查在动作前：`engineLink` 未恢复 → `backup`/`runtime` 报 `dependent files preserved`；
  `backup` 未恢复 → `runtime` 报 `Backup unresolved`；`link` 未恢复且指向诊断文件 → 诊断文件保留。
- 单步失败只进 `conflicts` 列表并继续；全部结束后若有冲突 → `Err("Recovery incomplete; journal retained: ...")`，
  记录文件保留；无冲突且非 `preview`/`engine_only` → 删记录。
- `backup` 分支严格区分三种情况：备份被改（保留）、原件已就位且内容一致（两边都保留）、原件被占（保留）。
- `runtime` 不校验哈希（游戏可能改写），`diagnostic`/`storage` 必须哈希一致才删。
- 归档分支只在 `engineLink` 记录为 `done` 且目标是**普通文件**时触发；`recovered` 目录建在记录所在目录下。
- 链接拆除统一走 `SymlinkOps::remove`，因此"链接只删链接本身"这条约束由平台层保证。

## 关卡 D（步 12–13）

| 条件 | 结果 |
|---|---|
| 3 款游戏 × 4 组参数 → 12 份诊断样本**逐字节**一致 | 通过 |
| 9 条识别边界（值变更、额外 section、重复键、注释改动、非法值、行尾空格、LF 换行） | 与 PS 结果一致 |
| 旧模板接受集合：鸣潮 4 份、异环/Steam 各 2 份，且互不认领 | 通过 |
| 发行 `dlssg_sm86.ini` 导入（11 键，`Path` 不出现） | 通过 |
| 渲染 → 重新导入 → 再渲染，设置与文本都不变 | 通过 |
| 自定义 INI 原样保留、标记行在最前、跳过结构化校验 | 通过 |
| 生成文本全 CRLF、标记行可取回 | 通过 |

## 步 12–13 的实现要点

- **12 键模型**：`Settings` 12 个字段；`Runtime.Path` 只在 `External`/`Pinned` 时落到文本里，
  因此文件是 11 或 12 行键值——"12 键"指模型，不是固定 12 行（已写进模块注释）。
- **默认值 3 / A / 1 / Off / 1**，与 0.4.2 发行配置一致（PARITY D-2：这是相对旧模板 `5/2/2` 的有意偏离）。
- **渲染格式**：两行注释头 → 六个分区（`General`/`FrameGeneration`/`DLSSG`/`SuperResolution`/
  `Logging`/`XeSSFG`/`Runtime` 顺序固定）→ 末尾标记行 `; dlssg-launcher-config=<uuid>`。
- **自定义 INI**：标记行插在最前，用户文本原样跟随，不做任何校验。
- **诊断生成**：字面替换顺序固定 `Optimized → Level → Preset → Mode(LF) → Mode(CRLF)`，
  顺序写错会互相污染（模板里同时有 `Optimized=2` 与 `Level=2`）。
- **诊断识别**：先提取 4 个值（每个键必须恰好一次、值必须在范围内），再用 0.4.2 模板重生成并整段比较。
  旧文本（0.4.0 / 0.4.0-mode-off / 0.3.2）在现有实现里是 `-ceq` 直比，因此
  `DiagnosticAcceptance` 把"可重生成"与"逐字匹配"分成两类，避免把旧文本误送进重生成路径。

## 下一步

步 14–15：`session`（三策略 + 启动器三阶段等待）与 `log`（结构化日志、24 小时轮转、组件日志游标）。




