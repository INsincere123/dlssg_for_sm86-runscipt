//! 日志与组件日志游标（Phase 1 步 15）。
//!
//! 对应现有实现：
//! - `Start-LauncherLog`（`ScriptLogging.ps1:2-19`）：建 `logs` 目录、清理 24 小时前的
//!   `*.log`（跳过重解析点、删除失败忽略），文件名带时间戳与 PID。
//! - 组件日志：`dlssg_sm86\logs\dlssg_sm86.log`，所有模块/进程共写一个文件，
//!   每行以本地时间、PID、EXE 名、级别字母开头，按 EXE 名或 PID 过滤。

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::{Error, Result};

/// 日志级别。字母与组件日志一致：`E`=1、`I`=2、`D`=3。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error = 1,
    Info = 2,
    Detail = 3,
}

impl Level {
    pub fn letter(self) -> char {
        match self {
            Level::Error => 'E',
            Level::Info => 'I',
            Level::Detail => 'D',
        }
    }

    pub fn as_number(self) -> u8 {
        self as u8
    }
}

/// 启动器日志文件。
pub struct LauncherLog {
    path: PathBuf,
    file: std::fs::File,
}

impl LauncherLog {
    /// 文件名规则：`<stem>_<yyyyMMdd_HHmmss_fff>_<pid>.log`（与现有实现一致，时间取 UTC）。
    pub fn file_name(stem: &str, now: SystemTime, pid: u32) -> String {
        let (year, month, day, hour, minute, second, millis) = parts(now);
        format!(
            "{stem}_{year:04}{month:02}{day:02}_{hour:02}{minute:02}{second:02}_{millis:03}_{pid}.log"
        )
    }

    /// 在 `logs` 目录里新建日志；先按 24 小时规则清理旧文件。
    pub fn open(root: &Path, stem: &str, now: SystemTime, pid: u32) -> Result<Self> {
        let directory = root.join("logs");
        std::fs::create_dir_all(&directory).map_err(|error| Error::io(&directory, error))?;
        let _ = cleanup_old_logs(&directory, now, Duration::from_secs(24 * 3600));
        let path = directory.join(Self::file_name(stem, now, pid));
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| Error::io(&path, error))?;
        let mut log = Self { path, file };
        log.write_line(&format!(
            "[{}] 运行日志：{}",
            Self::stamp(now),
            log.path.display()
        ))?;
        Ok(log)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 写一行：`[时间] 级别 消息`。
    pub fn write(&mut self, level: Level, message: &str) -> Result<()> {
        let line = format!(
            "[{}] {} {message}",
            Self::stamp(SystemTime::now()),
            level.letter()
        );
        self.write_line(&line)
    }

    fn write_line(&mut self, line: &str) -> Result<()> {
        self.file
            .write_all(format!("{line}\r\n").as_bytes())
            .map_err(|error| Error::io(&self.path, error))?;
        self.file
            .flush()
            .map_err(|error| Error::io(&self.path, error))
    }

    /// `yyyy-MM-dd HH:mm:ss.fff`（UTC）。
    pub fn stamp(now: SystemTime) -> String {
        let (year, month, day, hour, minute, second, millis) = parts(now);
        format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}.{millis:03}")
    }
}

/// 删除 `directory` 下修改时间早于 `max_age` 的普通 `*.log`。
///
/// 与现有实现一致：跳过重解析点，删除失败一律忽略，返回实际删除数量。
pub fn cleanup_old_logs(directory: &Path, now: SystemTime, max_age: Duration) -> Result<usize> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(Error::io(directory, error)),
    };
    let mut removed = 0_usize;
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        if path
            .extension()
            .and_then(|value| value.to_str())
            .is_none_or(|value| !value.eq_ignore_ascii_case("log"))
        {
            continue;
        }
        // 重解析点（链接）不动：删链接可能连带目标。
        if std::fs::symlink_metadata(&path)
            .map(|item| item.file_type().is_symlink())
            .unwrap_or(false)
        {
            continue;
        }
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        let age = now.duration_since(modified).unwrap_or_default();
        if age < max_age {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

/// 组件日志游标：只读取上次之后新增的内容。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComponentLogCursor {
    pub offset: u64,
    pub path: Option<PathBuf>,
}

impl ComponentLogCursor {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            offset: 0,
            path: Some(path.into()),
        }
    }

    /// 读新增行。文件被截断或替换时从头读。
    pub fn read_new(&mut self) -> Result<Vec<String>> {
        let Some(path) = self.path.clone() else {
            return Ok(Vec::new());
        };
        let mut file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(Error::io(&path, error)),
        };
        let length = file
            .metadata()
            .map_err(|error| Error::io(&path, error))?
            .len();
        if length < self.offset {
            self.offset = 0;
        }
        file.seek(SeekFrom::Start(self.offset))
            .map_err(|error| Error::io(&path, error))?;
        let mut text = String::new();
        file.read_to_string(&mut text)
            .map_err(|error| Error::io(&path, error))?;
        self.offset = length;
        Ok(text
            .lines()
            .map(|line| line.trim_end_matches('\r').to_owned())
            .collect())
    }
}

/// 组件日志行按 EXE 名或 PID 过滤。行格式：`时间 pid exe 级别 {json}`。
pub fn filter_component_lines<'a>(
    lines: &'a [String],
    exe: Option<&str>,
    pid: Option<u32>,
) -> Vec<&'a String> {
    lines
        .iter()
        .filter(|line| {
            // 行格式：`<日期> <时刻> <pid> <exe> <级别> {json}`，日期与时刻之间也有空格，
            // 因此按"第一个能解析成数字的字段 = pid，紧随其后 = exe"来定位。
            let fields: Vec<&str> = line.split_whitespace().collect();
            let Some(index) = fields.iter().position(|field| field.parse::<u32>().is_ok()) else {
                return false;
            };
            if index + 1 >= fields.len() {
                return false;
            }
            let line_pid = fields[index].parse::<u32>().ok();
            let line_exe = fields[index + 1];
            let pid_ok = pid.is_none_or(|wanted| line_pid == Some(wanted));
            let exe_ok = exe.is_none_or(|wanted| line_exe.eq_ignore_ascii_case(wanted));
            pid_ok && exe_ok
        })
        .collect()
}

fn parts(now: SystemTime) -> (i64, u32, u32, u32, u32, u32, u32) {
    let since_epoch = now.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);
    let seconds = since_epoch.as_secs() as i64;
    let millis = since_epoch.subsec_millis();
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = crate::journal_store::civil_from_days(days);
    let hour = (seconds_of_day / 3600) as u32;
    let minute = ((seconds_of_day % 3600) / 60) as u32;
    let second = (seconds_of_day % 60) as u32;
    (year, month, day, hour, minute, second, millis)
}

#[cfg(test)]
fn set_modified(path: &Path, when: SystemTime) {
    // 没有跨平台的 set_mtime 标准 API：用写回后立刻改文件时间的方式不可行，
    // 因此这里通过 Win32 的 SetFileTime 设置。
    use std::os::windows::io::AsRawHandle;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("打开文件");
    let handle = file.as_raw_handle() as isize;
    let seconds = when
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    // FILETIME: 100ns since 1601-01-01
    let ticks = (seconds + 11_644_473_600) as u64 * 10_000_000;
    let low = (ticks & 0xFFFF_FFFF) as u32;
    let high = (ticks >> 32) as u32;
    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    unsafe extern "system" {
        fn SetFileTime(
            handle: isize,
            creation: *const FileTime,
            access: *const FileTime,
            write: *const FileTime,
        ) -> i32;
    }
    let value = FileTime { low, high };
    let ok = unsafe { SetFileTime(handle, std::ptr::null(), std::ptr::null(), &value) };
    assert_ne!(ok, 0, "设置文件时间失败");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("dlssg-log-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("临时目录");
        directory
    }

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    #[test]
    fn file_name_and_stamp_use_utc_parts() {
        // 2026-11-22T13:56:09.523Z
        let now = UNIX_EPOCH + Duration::new(1_795_355_769, 523_000_000);
        assert_eq!(
            LauncherLog::file_name("启动鸣潮", now, 4242),
            "启动鸣潮_20261122_135609_523_4242.log"
        );
        assert_eq!(LauncherLog::stamp(now), "2026-11-22 13:56:09.523");
    }

    #[test]
    fn writing_appends_crlf_lines() {
        let directory = scratch("write");
        let now = at(1_795_355_769);
        let mut log = LauncherLog::open(&directory, "launcher", now, 7).expect("打开日志");
        log.write(Level::Info, "开始部署").expect("写日志");
        let text = std::fs::read_to_string(log.path()).expect("读日志");
        assert!(text.contains("运行日志："), "{text}");
        assert!(text.contains("开始部署"), "{text}");
        assert_eq!(text.matches("\r\n").count(), text.matches('\n').count());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn cleanup_removes_only_old_ordinary_logs() {
        let directory = scratch("cleanup");
        let now = at(10 * 86_400);
        let old = directory.join("old.log");
        let fresh = directory.join("fresh.log");
        let other = directory.join("keep.txt");
        std::fs::write(&old, "old").expect("旧日志");
        std::fs::write(&fresh, "fresh").expect("新日志");
        std::fs::write(&other, "text").expect("非日志");
        // 把 old.log 的修改时间改到 48 小时前
        let old_time = now - Duration::from_secs(48 * 3600);
        set_modified(&old, old_time);

        let removed =
            cleanup_old_logs(&directory, now, Duration::from_secs(24 * 3600)).expect("清理");
        assert_eq!(removed, 1);
        assert!(!old.exists());
        assert!(fresh.exists());
        assert!(other.exists());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn cleanup_missing_directory_is_not_an_error() {
        let missing = std::env::temp_dir().join("dlssg-log-no-such-dir-xyz");
        let _ = std::fs::remove_dir_all(&missing);
        assert_eq!(
            cleanup_old_logs(&missing, at(0), Duration::from_secs(3600)).expect("不应报错"),
            0
        );
    }

    #[test]
    fn cursor_reads_only_appended_lines_and_restarts_on_truncate() {
        let directory = scratch("cursor");
        let path = directory.join("dlssg_sm86.log");
        std::fs::write(&path, "line1\r\nline2\r\n").expect("写日志");

        let mut cursor = ComponentLogCursor::new(&path);
        assert_eq!(cursor.read_new().expect("读"), vec!["line1", "line2"]);
        assert!(cursor.read_new().expect("再读").is_empty());

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("追加");
        file.write_all(b"line3\r\n").expect("写");
        drop(file);
        assert_eq!(cursor.read_new().expect("读增量"), vec!["line3"]);

        std::fs::write(&path, "only\r\n").expect("截断重写");
        assert_eq!(cursor.read_new().expect("重建后从头读"), vec!["only"]);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn cursor_on_missing_file_is_empty() {
        let mut cursor = ComponentLogCursor::new(std::env::temp_dir().join("dlssg-no-log.log"));
        assert!(cursor.read_new().expect("读").is_empty());
    }

    #[test]
    fn component_lines_filter_by_exe_and_pid() {
        let lines = vec![
            "2026-11-22 13:56:09.523 4242 game.exe I {\"event\":\"a\"}".to_owned(),
            "2026-11-22 13:56:10.100 4242 other.exe E {\"event\":\"b\"}".to_owned(),
            "2026-11-22 13:56:11.200 9999 game.exe D {\"event\":\"c\"}".to_owned(),
            "garbage".to_owned(),
        ];
        assert_eq!(
            filter_component_lines(&lines, Some("game.exe"), None).len(),
            2
        );
        assert_eq!(
            filter_component_lines(&lines, Some("GAME.EXE"), Some(4242)).len(),
            1
        );
        assert_eq!(filter_component_lines(&lines, None, Some(9999)).len(), 1);
        assert_eq!(filter_component_lines(&lines, None, None).len(), 3);
    }
}
