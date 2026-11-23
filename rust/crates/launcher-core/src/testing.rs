//! 测试替身：让 core 的逻辑不依赖 Windows、也不要求管理员权限。
//!
//! 生产代码永远不引用本模块；它只为单元/集成测试提供可记录的假实现。

#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::error::{Error, Result};
use crate::ops::{
    Clock, LinkInfo, LinkKind, MutexHandle, MutexOps, ProcessEntry, ProcessProbe, SymlinkOps,
};

fn key(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

/// 内存链接表。从未登记过的路径一律视为"不是链接"。
#[derive(Default)]
pub struct FakeSymlinkOps {
    links: RefCell<HashMap<String, LinkInfo>>,
    removed: RefCell<Vec<PathBuf>>,
    /// 设置后 `create` 立即失败，用于验证失败路径不留下半成品。
    pub fail_create: RefCell<Option<String>>,
}

impl FakeSymlinkOps {
    pub fn new() -> Self {
        Self::default()
    }

    /// 预置一个已存在的链接。
    pub fn with_link(self, link: impl AsRef<Path>, info: LinkInfo) -> Self {
        self.links.borrow_mut().insert(key(link.as_ref()), info);
        self
    }

    /// 预置一个目录联接（junction）：恢复流程必须拒绝把它当成本项目的链接。
    pub fn with_junction(self, link: impl AsRef<Path>, target: impl AsRef<Path>) -> Self {
        self.links.borrow_mut().insert(
            key(link.as_ref()),
            LinkInfo {
                kind: LinkKind::Junction,
                target: target.as_ref().to_path_buf(),
                target_is_dir: true,
            },
        );
        self
    }

    pub fn removed(&self) -> Vec<PathBuf> {
        self.removed.borrow().clone()
    }

    pub fn count(&self) -> usize {
        self.links.borrow().len()
    }
}

impl SymlinkOps for FakeSymlinkOps {
    fn create(&self, link: &Path, target: &Path, target_is_dir: bool) -> Result<()> {
        if let Some(message) = self.fail_create.borrow().clone() {
            return Err(Error::invalid(message));
        }
        self.links.borrow_mut().insert(
            key(link),
            LinkInfo {
                kind: LinkKind::SymbolicLink,
                target: target.to_path_buf(),
                target_is_dir,
            },
        );
        Ok(())
    }

    fn inspect(&self, path: &Path) -> Result<Option<LinkInfo>> {
        Ok(self.links.borrow().get(&key(path)).cloned())
    }

    fn remove(&self, path: &Path) -> Result<()> {
        self.links.borrow_mut().remove(&key(path));
        self.removed.borrow_mut().push(path.to_path_buf());
        Ok(())
    }
}

/// 记录每次 acquire/release 的互斥替身。
#[derive(Default)]
pub struct FakeMutexOps {
    next: RefCell<u64>,
    held: RefCell<HashSet<String>>,
    pub acquisitions: RefCell<Vec<(String, Duration)>>,
    pub releases: RefCell<Vec<String>>,
    /// 设置后 `acquire` 对**该名字**一律返回超时。
    pub blocked: RefCell<Option<String>>,
}

impl FakeMutexOps {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn blocked_on(name: &str) -> Self {
        let fake = Self::new();
        *fake.blocked.borrow_mut() = Some(name.to_owned());
        fake
    }

    pub fn is_held(&self, name: &str) -> bool {
        self.held.borrow().contains(name)
    }
}

impl MutexOps for FakeMutexOps {
    fn acquire(&self, name: &str, wait: Duration) -> Result<MutexHandle> {
        self.acquisitions.borrow_mut().push((name.to_owned(), wait));
        if self.blocked.borrow().as_deref() == Some(name) || self.held.borrow().contains(name) {
            return Err(Error::MutexTimeout {
                name: name.to_owned(),
                wait_ms: wait.as_millis() as u64,
            });
        }
        self.held.borrow_mut().insert(name.to_owned());
        let mut next = self.next.borrow_mut();
        *next += 1;
        Ok(MutexHandle(*next))
    }

    fn release(&self, _handle: MutexHandle) -> Result<()> {
        let names: Vec<String> = self.held.borrow().iter().cloned().collect();
        if let Some(name) = names.into_iter().next() {
            self.held.borrow_mut().remove(&name);
            self.releases.borrow_mut().push(name);
        }
        Ok(())
    }
}

/// 固定进程表。
pub struct FakeProcessProbe {
    pub entries: Vec<ProcessEntry>,
    pub current: ProcessEntry,
}

impl FakeProcessProbe {
    pub fn new(entries: Vec<ProcessEntry>, current: ProcessEntry) -> Self {
        Self { entries, current }
    }

    pub fn empty() -> Self {
        let current = ProcessEntry {
            pid: 4242,
            name: "DlssgLauncher".to_owned(),
            executable: Some(PathBuf::from(r"D:\launcher\DlssgLauncher.exe")),
            started_at: Some(SystemTime::UNIX_EPOCH),
        };
        Self {
            entries: vec![current.clone()],
            current,
        }
    }

    pub fn with_entry(mut self, entry: ProcessEntry) -> Self {
        self.entries.push(entry);
        self
    }
}

impl ProcessProbe for FakeProcessProbe {
    fn list(&self) -> Result<Vec<ProcessEntry>> {
        Ok(self.entries.clone())
    }

    fn current(&self) -> Result<ProcessEntry> {
        Ok(self.current.clone())
    }

    fn started_at(&self, pid: u32) -> Result<SystemTime> {
        self.entries
            .iter()
            .find(|entry| entry.pid == pid)
            .and_then(|entry| entry.started_at)
            .ok_or_else(|| Error::invalid(format!("进程 {pid} 没有启动时间")))
    }
}

/// 固定时间。
pub struct FixedClock(pub SystemTime);

impl FixedClock {
    /// 2026-11-22T00:00:00Z，便于快照比对。
    pub const EPOCH_2026_11_22: u64 = 1_795_305_600;

    pub fn at_epoch_seconds(seconds: u64) -> Self {
        Self(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
    }
}

impl Default for FixedClock {
    fn default() -> Self {
        Self::at_epoch_seconds(Self::EPOCH_2026_11_22)
    }
}

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}
