//! `launcher-core`：与 UI 无关的部署/恢复内核。
//!
//! 当前进度：**M0 已冻结契约**（[PARITY.md](../docs/PARITY.md)），Phase 1 逐步实现。
//! 本 crate 不引用任何 Win32 或 UI 依赖；所有平台能力通过 [`ops`] 的 trait 注入。
//!
//! 模块顺序（与 `rust开发.md` Phase 1 的步号对应）：
//!
//! - 步 1：错误类型、能力 trait、样本读取器（当前）
//! - 步 2：`path`
//! - 步 3–5：`config`
//! - 步 6–7：`journal`
//! - 步 8–9：`resources`
//! - 步 10–11：`restore`
//! - 步 12–13：`settings`
//! - 步 14：`session`
//! - 步 15：`log`

pub mod config;
pub mod diagnostic;
pub mod direct;
pub mod discovery;
pub mod engine_config;
pub mod error;
pub mod fixtures;
pub mod journal;
pub mod journal_store;
pub mod log;
pub mod ops;
pub mod path;
pub mod pe;
pub mod preferences;
pub mod prefs;
pub mod quality;
pub mod resources;
pub mod restore;
pub mod session;
pub mod settings;
pub mod steam;
#[doc(hidden)]
pub mod testing;

pub use error::{Error, Result};
pub use ops::{
    Clock, LinkInfo, LinkKind, MutexHandle, MutexLease, MutexOps, Platform, ProcessEntry,
    ProcessProbe, SymlinkOps,
};

/// crate 版本，写进日志便于排查。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// `dlssg_sm86` 组件基线。当前仓库使用 0.4.2 资源布局（runtime/backend/kernels/xefg/sr），
/// 界面「关于」与便携包清单都以它为口径。
pub const COMPONENT_BASELINE: &str = "0.4.2";
