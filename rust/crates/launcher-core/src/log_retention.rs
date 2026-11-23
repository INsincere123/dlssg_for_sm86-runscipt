//! 日志保留：只清理 `logs` 目录里超过保留期的 `*.log`。
//!
//! 约束（用户确认的行为）：
//!
//! - 只删**普通文件**，且扩展名恰好是 `log`（大小写不敏感）
//! - 不递归，不进子目录
//! - 不动 `ui-run.json` / `ui-restore.json` / `status.json` 等报告，也不动日志目录以外的任何东西
//! - 修改时间读不到、或时间在未来（时钟回拨）时一律保留：宁可留旧日志，也不误删
//! - 单个文件删除失败只记录，不影响其它文件
//!
//! 这里不取"当前时间"，由调用方注入 [`SystemTime`]，因此可以在无时钟环境下测试。

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// 默认保留天数。
pub const RETENTION_DAYS: u64 = 7;

/// 一次清理的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PruneReport {
    /// 已删除的日志文件（完整路径，按名字排序）。
    pub removed: Vec<PathBuf>,
    /// 未到保留期、或被跳过的日志文件数。
    pub kept: usize,
    /// 目录不存在（首次启动）时为 `true`，不算错误。
    pub absent: bool,
    /// 读目录或删文件的失败说明。
    pub errors: Vec<String>,
}

impl PruneReport {
    /// 便于日志落地的一行摘要。
    pub fn summary(&self) -> String {
        if self.absent {
            return "日志目录不存在，跳过保留期清理".to_owned();
        }
        let mut text = format!(
            "日志保留期清理：删除 {} 个，保留 {} 个",
            self.removed.len(),
            self.kept
        );
        if !self.errors.is_empty() {
            text.push_str(&format!("，失败 {} 个", self.errors.len()));
        }
        text
    }
}

/// 清理 `directory` 下超过 `days` 天的 `*.log`。
pub fn prune(directory: &Path, now: SystemTime, days: u64) -> PruneReport {
    let mut report = PruneReport::default();
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            report.absent = true;
            return report;
        }
        Err(error) => {
            report
                .errors
                .push(format!("无法读取日志目录 {}：{error}", directory.display()));
            return report;
        }
    };

    let mut candidates: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                report.errors.push(format!("无法读取日志条目：{error}"));
                continue;
            }
        };
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !name.to_ascii_lowercase().ends_with(".log") {
            continue;
        }
        match std::fs::symlink_metadata(&path) {
            // 链接不跟随：日志目录里出现链接本身就异常，保留现场。
            Ok(metadata) if metadata.file_type().is_symlink() => {
                report.kept += 1;
                continue;
            }
            Ok(metadata) if !metadata.is_file() => continue,
            Ok(_) => candidates.push(path),
            Err(error) => {
                report
                    .errors
                    .push(format!("无法读取 {}：{error}", path.display()));
            }
        }
    }
    candidates.sort();

    let retention = Duration::from_secs(days.saturating_mul(24 * 60 * 60));
    for path in candidates {
        let modified = match std::fs::metadata(&path).and_then(|metadata| metadata.modified()) {
            Ok(modified) => modified,
            Err(error) => {
                report
                    .errors
                    .push(format!("无法读取修改时间 {}：{error}", path.display()));
                report.kept += 1;
                continue;
            }
        };
        // 时钟回拨（修改时间在未来）时 `duration_since` 报错，按"保留"处理。
        let age = match now.duration_since(modified) {
            Ok(age) => age,
            Err(_) => {
                report.kept += 1;
                continue;
            }
        };
        if age < retention {
            report.kept += 1;
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => report.removed.push(path),
            Err(error) => {
                report.kept += 1;
                report
                    .errors
                    .push(format!("无法删除 {}：{error}", path.display()));
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tree(PathBuf);

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tree(name: &str) -> Tree {
        let root = std::env::temp_dir().join(format!("dlssg-logs-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("临时目录");
        Tree(root)
    }

    fn write(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("父目录");
        }
        std::fs::write(path, b"x").expect("写文件");
    }

    /// 把文件的修改时间往前推 `days` 天（用 `File::set_times` 不可用，改走系统时间估算）。
    fn age_file(path: &Path, days: u64) {
        // 只能"变旧"：把 mtime 设成 now - days。标准库在 Windows 上通过 SetFileTime 生效。
        let target = SystemTime::now() - Duration::from_secs(days * 24 * 60 * 60);
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("打开文件");
        file.set_modified(target).expect("设置修改时间");
    }

    #[test]
    fn only_expired_logs_are_removed() {
        let tree = tree("expired");
        let fresh = tree.0.join("engine_fresh.log");
        let old = tree.0.join("engine_old.log");
        let report_json = tree.0.join("ui-run.json");
        let other = tree.0.join("notes.txt");
        write(&fresh);
        write(&old);
        write(&report_json);
        write(&other);
        age_file(&fresh, 1);
        age_file(&old, 30);
        age_file(&report_json, 30);
        age_file(&other, 30);

        let report = prune(&tree.0, SystemTime::now(), RETENTION_DAYS);
        assert_eq!(report.removed, vec![old.clone()], "{report:?}");
        assert_eq!(report.kept, 1, "只统计 .log");
        assert!(report.errors.is_empty(), "{report:?}");
        assert!(!old.exists(), "旧日志应被删除");
        assert!(fresh.exists(), "新日志必须保留");
        assert!(report_json.exists(), "报告 JSON 不能被清理");
        assert!(other.exists(), "非日志文件不能被清理");
    }

    #[test]
    fn missing_directory_is_not_an_error() {
        let root = std::env::temp_dir().join("dlssg-logs-not-here-xyz");
        let _ = std::fs::remove_dir_all(&root);
        let report = prune(&root, SystemTime::now(), RETENTION_DAYS);
        assert!(report.absent);
        assert!(report.removed.is_empty());
        assert!(report.errors.is_empty());
        assert!(report.summary().contains("不存在"));
    }

    #[test]
    fn future_timestamps_are_kept() {
        let tree = tree("future");
        let path = tree.0.join("clock_skew.log");
        write(&path);
        // now 比文件更早：相当于时钟回拨，必须保留。
        let past = SystemTime::now() - Duration::from_secs(24 * 60 * 60 * 30);
        let report = prune(&tree.0, past, RETENTION_DAYS);
        assert!(report.removed.is_empty(), "{report:?}");
        assert_eq!(report.kept, 1);
        assert!(path.exists());
    }

    #[test]
    fn directories_and_sublogs_are_left_alone() {
        let tree = tree("nested");
        let nested = tree.0.join("archive.log");
        std::fs::create_dir_all(&nested).expect("子目录");
        let inner = nested.join("old.log");
        write(&inner);
        age_file(&inner, 60);
        let report = prune(&tree.0, SystemTime::now(), RETENTION_DAYS);
        assert!(report.removed.is_empty(), "不递归：{report:?}");
        assert!(inner.exists());
    }
}
