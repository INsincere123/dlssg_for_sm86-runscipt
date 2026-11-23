//! 平台能力边界。
//!
//! core 不直接调用 Win32：链接、互斥、进程与时间都通过这里的 trait 注入。
//! 真实现放在 `launcher-win`（M2），测试用内存实现，因此这些签名在关卡 A
//! 之后**不再修改**——`resources`/`restore`/`session` 都挂在它们之上。
//!
//! 与现有 PowerShell 实现的对应关系见 `rust/docs/PARITY.md`：
//! - [`SymlinkOps`] ↔ `New-Item -ItemType SymbolicLink`、`$item.LinkType`、`Remove-Item`
//! - [`MutexOps`] ↔ `[Threading.Mutex]::new`、`WaitOne`、`ReleaseMutex`
//! - [`ProcessProbe`] ↔ `Get-Process`、`(Get-Process -Id $PID).StartTime`
//! - [`Clock`] ↔ `Get-Date`

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::error::{Error, Result};

/// 链接类别。
///
/// PowerShell 用 `$item.LinkType` 区分：只有 `SymbolicLink` 才是本项目创建的链接，
/// junction 与其他重解析点一律按"不是我们的链接"处理（`DeploymentJournal.ps1:453`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    /// 文件或目录符号链接。
    SymbolicLink,
    /// 目录联接（mklink /J）。恢复流程拒绝把它当成 Engine.ini 链接。
    Junction,
    /// 其他重解析点（例如 OneDrive 占位符）。
    OtherReparse,
}

/// 一次链接检查的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkInfo {
    pub kind: LinkKind,
    /// 链接目标。相对链接保留原样，由调用方的路径归一化处理。
    pub target: PathBuf,
    /// **目标**是否是目录（不是链接本身）。
    ///
    /// Windows 上要按目标判断：`symlink_metadata` 取到的是链接自身，
    /// 对目录符号链接 `is_dir()` 永远是 `false`；删除目录符号链接必须用
    /// `RemoveDirectory`（Rust 的 `remove_dir`），用 `DeleteFile` 会失败。
    /// 这不是理论问题：M1 的真实链接用例第一次就踩在这里。
    pub target_is_dir: bool,
}

/// 创建、识别与移除链接。
///
/// Windows 真实现（M2，`launcher-win`）必须注意两点：
/// 1. **链接类别按重解析标签判断**：`SymbolicLink` 与 `Junction` 在文件系统里
///    都是重解析点，但恢复流程只认前者（PARITY C6，对应 PS 的 `LinkType -ne ''SymbolicLink''`）。
/// 2. **删除目录符号链接用 `RemoveDirectory`**，删除文件符号链接用 `DeleteFile`；
///    判断依据是 [`LinkInfo::target_is_dir`]，不是链接自身的属性。
pub trait SymlinkOps {
    /// 创建符号链接。`target_is_dir` 对应 PS 里按目标类型选 `-ItemType` 的行为。
    fn create(&self, link: &Path, target: &Path, target_is_dir: bool) -> Result<()>;

    /// 读取链接信息；路径不存在、是普通文件或普通目录时返回 `Ok(None)`。
    ///
    /// 注意：普通目录也要返回 `None` —— 调用方据此区分"被游戏替换成普通文件"
    /// （归档分支）与"链接仍在"（正常拆除）。
    fn inspect(&self, path: &Path) -> Result<Option<LinkInfo>>;

    /// 只移除链接本身，绝不递归进入目标目录。
    fn remove(&self, path: &Path) -> Result<()>;
}

/// 具名互斥句柄。不透明，由实现方定义含义。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MutexHandle(pub u64);

/// 具名互斥。
///
/// 与 PS 的对应：`WaitOne(0)`（抢不到立即失败）与 `WaitOne(5000)`（配置写回）。
/// 超时必须返回 [`Error::MutexTimeout`]，调用方据此区分"别人正在操作"与"系统错误"。
pub trait MutexOps {
    fn acquire(&self, name: &str, wait: Duration) -> Result<MutexHandle>;
    fn release(&self, handle: MutexHandle) -> Result<()>;
}

/// 进程快照条目。字段对应 `Get-Process` 用到的三列。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessEntry {
    pub pid: u32,
    /// 不含 `.exe` 的进程名，等价 PS 的 `ProcessName`（大小写不敏感比较）。
    pub name: String,
    /// 完整 EXE 路径。权限不足时可能是 `None`；此时不能按路径判定。
    pub executable: Option<PathBuf>,
    pub started_at: Option<SystemTime>,
}

/// 进程枚举与当前进程信息。
pub trait ProcessProbe {
    fn list(&self) -> Result<Vec<ProcessEntry>>;
    /// 当前进程条目，用于 journal 的 `ScriptPid` 与 `ScriptStarted`。
    fn current(&self) -> Result<ProcessEntry>;
    /// 指定 PID 的启动时间，等价 `(Get-Process -Id $PID).StartTime`。
    fn started_at(&self, pid: u32) -> Result<SystemTime>;
}

/// 时间来源。测试里可以固定时间，让 journal 快照可比对。
pub trait Clock {
    fn now(&self) -> SystemTime;
}

/// 四个能力打包成一个注入点，避免每个函数都带四个参数。
pub struct Platform<'a> {
    pub symlink: &'a dyn SymlinkOps,
    pub mutex: &'a dyn MutexOps,
    pub process: &'a dyn ProcessProbe,
    pub clock: &'a dyn Clock,
}

/// 互斥的 RAII 包装。
///
/// PS 依赖 `finally` 里的 `ReleaseMutex`；Rust 侧用 `Drop` 保证异常路径也会释放，
/// 显式 [`MutexLease::release`] 则能把释放失败上报。
pub struct MutexLease<'a> {
    ops: &'a dyn MutexOps,
    handle: MutexHandle,
    name: String,
    released: bool,
}

impl<'a> MutexLease<'a> {
    pub fn acquire(ops: &'a dyn MutexOps, name: &str, wait: Duration) -> Result<Self> {
        let handle = ops.acquire(name, wait)?;
        Ok(Self {
            ops,
            handle,
            name: name.to_owned(),
            released: false,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// 主动释放并上报失败。之后的 `Drop` 不再重复释放。
    pub fn release(mut self) -> Result<()> {
        self.released = true;
        self.ops.release(self.handle)
    }
}

impl Drop for MutexLease<'_> {
    fn drop(&mut self) {
        if !self.released {
            self.released = true;
            let _ = self.ops.release(self.handle);
        }
    }
}

impl std::fmt::Debug for MutexLease<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MutexLease")
            .field("name", &self.name)
            .field("handle", &self.handle)
            .field("released", &self.released)
            .finish()
    }
}

/// 便于在日志里打印 `Error` 而不吞掉来源。
pub fn describe(error: &Error) -> String {
    error.to_string()
}
