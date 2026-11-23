//! `launcher-win`：`launcher-core` 的四个能力 trait 的 Windows 真实现（Phase 1 步 M2-01）。
//!
//! 不依赖任何第三方 crate：Win32 一律用 `extern "system"` 直接声明，避免给最终 EXE
//! 引入绑定层。三个约定（都来自 PARITY C6 与 M1 的实机教训）：
//!
//! 1. 链接类别按**重解析标签**判断：`IO_REPARSE_TAG_SYMLINK` 才是 `SymbolicLink`，
//!    `IO_REPARSE_TAG_MOUNT_POINT` 是 junction，恢复流程必须拒绝后者。
//! 2. 删除目录符号链接用 `RemoveDirectoryW`，文件符号链接用 `DeleteFileW`；
//!    判断依据是**目标**是否为目录。
//! 3. 互斥是线程所有的：`ReleaseMutex` 必须由获取它的线程调用（引擎是单线程，够用）。

pub mod elevate;
pub mod ipc;
pub mod registry;

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use launcher_core::ops::{
    Clock, LinkInfo, LinkKind, MutexHandle, MutexOps, ProcessEntry, ProcessProbe, SymlinkOps,
};
use launcher_core::{Error, Result};

type Handle = isize;

const INVALID_HANDLE_VALUE: Handle = -1;
const GENERIC_READ: u32 = 0x8000_0000;
const FILE_SHARE_ALL: u32 = 0x1 | 0x2 | 0x4;
const OPEN_EXISTING: u32 = 3;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
const FSCTL_GET_REPARSE_POINT: u32 = 0x0009_00A8;
const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;
const IO_REPARSE_TAG_SYMLINK: u32 = 0xA000_000C;
const SYMBOLIC_LINK_FLAG_DIRECTORY: u32 = 0x1;
const SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE: u32 = 0x2;
const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_ABANDONED: u32 = 0x80;
const WAIT_TIMEOUT: u32 = 0x102;
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_PATH_NOT_FOUND: u32 = 3;

#[repr(C)]
struct FileTime {
    low: u32,
    high: u32,
}

#[repr(C)]
struct ProcessEntry32W {
    size: u32,
    usage: u32,
    process_id: u32,
    default_heap_id: usize,
    module_id: u32,
    threads: u32,
    parent_process_id: u32,
    pri_class_base: i32,
    flags: u32,
    exe_file: [u16; 260],
}

#[repr(C)]
struct SecurityAttributes {
    length: u32,
    descriptor: *mut c_void,
    inherit: i32,
}

use std::os::windows::ffi::OsStrExt;

fn wide_str(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateSymbolicLinkW(link: *const u16, target: *const u16, flags: u32) -> u8;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        attributes: *const SecurityAttributes,
        disposition: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    fn GetFileAttributesW(name: *const u16) -> u32;
    fn DeviceIoControl(
        device: Handle,
        code: u32,
        input: *mut c_void,
        input_size: u32,
        output: *mut c_void,
        output_size: u32,
        returned: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn RemoveDirectoryW(name: *const u16) -> i32;
    fn DeleteFileW(name: *const u16) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn CreateMutexW(attributes: *const SecurityAttributes, owner: i32, name: *const u16) -> Handle;
    fn ReleaseMutex(handle: Handle) -> i32;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> Handle;
    fn Process32FirstW(snapshot: Handle, entry: *mut ProcessEntry32W) -> i32;
    fn Process32NextW(snapshot: Handle, entry: *mut ProcessEntry32W) -> i32;
    fn OpenProcess(access: u32, inherit: i32, process_id: u32) -> Handle;
    fn QueryFullProcessImageNameW(
        process: Handle,
        flags: u32,
        buffer: *mut u16,
        size: *mut u32,
    ) -> i32;
    fn GetProcessTimes(
        process: Handle,
        creation: *mut FileTime,
        exit: *mut FileTime,
        kernel: *mut FileTime,
        user: *mut FileTime,
    ) -> i32;
    fn GetCurrentProcessId() -> u32;
    fn GetConsoleWindow() -> Handle;
}

fn last_error() -> Error {
    Error::io_plain(std::io::Error::last_os_error())
}

fn path_wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn file_time_to_system_time(time: FileTime) -> SystemTime {
    let ticks = ((time.high as u64) << 32) | time.low as u64;
    // FILETIME 是自 1601-01-01 起的 100ns 计数。
    const EPOCH_DIFFERENCE: u64 = 11_644_473_600;
    let seconds = ticks / 10_000_000;
    let nanos = (ticks % 10_000_000) * 100;
    if seconds < EPOCH_DIFFERENCE {
        return UNIX_EPOCH;
    }
    UNIX_EPOCH + Duration::new(seconds - EPOCH_DIFFERENCE, nanos as u32)
}

// ---------------------------------------------------------------- 链接

/// 真实文件系统链接操作。
#[derive(Debug, Default, Clone, Copy)]
pub struct WinSymlinkOps;

impl SymlinkOps for WinSymlinkOps {
    fn create(&self, link: &Path, target: &Path, target_is_dir: bool) -> Result<()> {
        let mut flags = SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE;
        if target_is_dir {
            flags |= SYMBOLIC_LINK_FLAG_DIRECTORY;
        }
        let link_wide = path_wide(link);
        let target_wide = path_wide(target);
        let ok = unsafe { CreateSymbolicLinkW(link_wide.as_ptr(), target_wide.as_ptr(), flags) };
        if ok == 0 {
            return Err(Error::io(link, std::io::Error::last_os_error()));
        }
        Ok(())
    }

    fn inspect(&self, path: &Path) -> Result<Option<LinkInfo>> {
        let wide = path_wide(path);
        let attributes = unsafe { GetFileAttributesW(wide.as_ptr()) };
        if attributes == u32::MAX {
            let code = last_error_code();
            if code == ERROR_FILE_NOT_FOUND || code == ERROR_PATH_NOT_FOUND {
                return Ok(None);
            }
            return Err(Error::io(
                path,
                std::io::Error::from_raw_os_error(code as i32),
            ));
        }
        if attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
            return Ok(None);
        }

        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_ALL,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                0,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(Error::io(path, std::io::Error::last_os_error()));
        }
        let mut buffer = vec![0u8; 16 * 1024];
        let mut returned: u32 = 0;
        let ok = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_GET_REPARSE_POINT,
                std::ptr::null_mut(),
                0,
                buffer.as_mut_ptr() as *mut c_void,
                buffer.len() as u32,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        unsafe { CloseHandle(handle) };
        if ok == 0 {
            return Err(Error::io(path, std::io::Error::last_os_error()));
        }
        let data = &buffer[..returned as usize];
        parse_reparse(path, data)
    }

    fn remove(&self, path: &Path) -> Result<()> {
        let info = self.inspect(path)?.ok_or_else(|| {
            Error::invalid(format!("不是链接，拒绝按链接删除：{}", path.display()))
        })?;
        let wide = path_wide(path);
        let ok = unsafe {
            if info.target_is_dir {
                RemoveDirectoryW(wide.as_ptr())
            } else {
                DeleteFileW(wide.as_ptr())
            }
        };
        if ok == 0 {
            return Err(Error::io(path, std::io::Error::last_os_error()));
        }
        Ok(())
    }
}

fn last_error_code() -> u32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32
}

/// 解析 `REPARSE_DATA_BUFFER`。返回 `None` 表示不是链接类重解析点。
fn parse_reparse(path: &Path, data: &[u8]) -> Result<Option<LinkInfo>> {
    if data.len() < 8 {
        return Ok(None);
    }
    let tag = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let kind = match tag {
        IO_REPARSE_TAG_SYMLINK => LinkKind::SymbolicLink,
        IO_REPARSE_TAG_MOUNT_POINT => LinkKind::Junction,
        _ => {
            return Ok(Some(LinkInfo {
                kind: LinkKind::OtherReparse,
                target: target_path_of(data, false).unwrap_or_default(),
                target_is_dir: false,
            }));
        }
    };
    let is_symlink = kind == LinkKind::SymbolicLink;
    let target = target_path_of(data, is_symlink)
        .ok_or_else(|| Error::invalid(format!("重解析数据没有可解析的目标：{}", path.display())))?;
    let attribute = unsafe { GetFileAttributesW(path_wide(&target).as_ptr()) };
    let target_is_dir = attribute != u32::MAX && attribute & FILE_ATTRIBUTE_DIRECTORY != 0;
    Ok(Some(LinkInfo {
        kind,
        target,
        target_is_dir,
    }))
}

/// 从重解析数据里取 `SubstituteName`（`\??\C:\...` → `C:\...`）。
fn target_path_of(data: &[u8], is_symlink: bool) -> Option<PathBuf> {
    // 偏移表起点：SYMLINK 有 4 个 u16（含 Flags），MOUNT_POINT 也有 4 个 u16。
    let (offset_pos, buffer_pos) = if is_symlink { (8, 20) } else { (8, 16) };
    let substitute_offset =
        u16::from_le_bytes([*data.get(offset_pos)?, *data.get(offset_pos + 1)?]) as usize;
    let substitute_length =
        u16::from_le_bytes([*data.get(offset_pos + 2)?, *data.get(offset_pos + 3)?]) as usize;
    let start = buffer_pos + substitute_offset;
    let end = start + substitute_length;
    if end > data.len() || substitute_length % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = data[start..end]
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let text = String::from_utf16_lossy(&units);
    let stripped = text
        .strip_prefix("\\??\\")
        .or_else(|| text.strip_prefix("\\\\?\\"))
        .unwrap_or(&text);
    let final_text = match stripped.strip_prefix("UNC\\") {
        Some(rest) => format!("\\\\{rest}"),
        None => stripped.to_owned(),
    };
    Some(PathBuf::from(final_text))
}

// ---------------------------------------------------------------- 互斥

/// 真实具名互斥。句柄直接编码进 [`MutexHandle`]。
#[derive(Debug, Default, Clone, Copy)]
pub struct WinMutexOps;

impl MutexOps for WinMutexOps {
    fn acquire(&self, name: &str, wait: Duration) -> Result<MutexHandle> {
        let wide = wide_str(name);
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, wide.as_ptr()) };
        if handle == 0 || handle == INVALID_HANDLE_VALUE {
            return Err(last_error());
        }
        let milliseconds = u32::try_from(wait.as_millis()).unwrap_or(u32::MAX);
        let result = unsafe { WaitForSingleObject(handle, milliseconds) };
        match result {
            WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(MutexHandle(handle as u64)),
            WAIT_TIMEOUT => {
                unsafe { CloseHandle(handle) };
                Err(Error::MutexTimeout {
                    name: name.to_owned(),
                    wait_ms: wait.as_millis() as u64,
                })
            }
            _ => {
                let error = last_error();
                unsafe { CloseHandle(handle) };
                Err(error)
            }
        }
    }

    fn release(&self, handle: MutexHandle) -> Result<()> {
        let raw = handle.0 as Handle;
        let released = unsafe { ReleaseMutex(raw) };
        unsafe { CloseHandle(raw) };
        if released == 0 {
            return Err(last_error());
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- 进程

/// 真实进程枚举。`name` 去掉 `.exe` 后缀，与 PS 的 `ProcessName` 对齐。
#[derive(Debug, Default, Clone, Copy)]
pub struct WinProcessProbe;

impl ProcessProbe for WinProcessProbe {
    fn list(&self) -> Result<Vec<ProcessEntry>> {
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(last_error());
        }
        let mut entries = Vec::new();
        let mut raw: ProcessEntry32W = unsafe { std::mem::zeroed() };
        raw.size = std::mem::size_of::<ProcessEntry32W>() as u32;
        let mut ok = unsafe { Process32FirstW(snapshot, &mut raw) };
        while ok != 0 {
            entries.push(ProcessEntry {
                pid: raw.process_id,
                name: process_name(&raw),
                executable: executable_path(raw.process_id),
                started_at: started_at(raw.process_id).ok(),
            });
            ok = unsafe { Process32NextW(snapshot, &mut raw) };
        }
        unsafe { CloseHandle(snapshot) };
        Ok(entries)
    }

    fn current(&self) -> Result<ProcessEntry> {
        let pid = unsafe { GetCurrentProcessId() };
        Ok(ProcessEntry {
            pid,
            name: executable_path(pid)
                .as_deref()
                .and_then(|path| path.file_stem())
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| "unknown".to_owned()),
            executable: executable_path(pid),
            started_at: started_at(pid).ok(),
        })
    }

    fn started_at(&self, pid: u32) -> Result<SystemTime> {
        started_at(pid)
    }
}

fn process_name(entry: &ProcessEntry32W) -> String {
    let end = entry
        .exe_file
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(entry.exe_file.len());
    let text = String::from_utf16_lossy(&entry.exe_file[..end]);
    text.strip_suffix(".exe")
        .or_else(|| text.strip_suffix(".EXE"))
        .unwrap_or(&text)
        .to_owned()
}

fn executable_path(pid: u32) -> Option<PathBuf> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle == 0 {
        return None;
    }
    let mut buffer = vec![0u16; 1024];
    let mut size = buffer.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut size) };
    unsafe { CloseHandle(handle) };
    if ok == 0 {
        return None;
    }
    Some(PathBuf::from(String::from_utf16_lossy(
        &buffer[..size as usize],
    )))
}

fn started_at(pid: u32) -> Result<SystemTime> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle == 0 {
        return Err(last_error());
    }
    let mut creation: FileTime = unsafe { std::mem::zeroed() };
    let mut exit: FileTime = unsafe { std::mem::zeroed() };
    let mut kernel: FileTime = unsafe { std::mem::zeroed() };
    let mut user: FileTime = unsafe { std::mem::zeroed() };
    let ok = unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) };
    unsafe { CloseHandle(handle) };
    if ok == 0 {
        return Err(last_error());
    }
    Ok(file_time_to_system_time(creation))
}

// ---------------------------------------------------------------- 时钟

/// 真实系统时钟。
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// 当前进程是否处于高完整性（提权）上下文。
pub fn is_elevated() -> bool {
    std::process::Command::new("cmd")
        .args(["/c", "net", "session"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// 当前进程的控制台窗口句柄；`0` 表示没有控制台（`windows` 子系统或 `CREATE_NO_WINDOW`）。
pub fn console_window() -> isize {
    unsafe { GetConsoleWindow() }
}

/// 便于日志：把 `Path` 转成可读文本。
pub fn display(path: &Path) -> String {
    path.display().to_string()
}

#[allow(dead_code)]
const _: u32 = FILE_ATTRIBUTE_NORMAL;
#[allow(dead_code)]
const _: u32 = SYMBOLIC_LINK_FLAG_DIRECTORY;

// ---------------------------------------------------------------- 单会话（M2-05）

/// 全局单会话互斥名（PARITY 新增规则 N-1）。
pub const SINGLE_SESSION_MUTEX: &str = "Global\\DlssgSession";

/// 已持有的单会话锁。`Drop` 时释放。
#[derive(Debug)]
pub struct SingleSession {
    handle: MutexHandle,
}

impl Drop for SingleSession {
    fn drop(&mut self) {
        let _ = WinMutexOps.release(self.handle);
    }
}

/// 尝试取得单会话锁：`Ok(None)` 表示已有另一个会话在跑（第二款游戏必须被拒绝）。
///
/// Win32 具名互斥是**线程所有且可递归**的，所以"同线程再取一次"会成功；
/// 真正的排斥发生在另一个线程/进程上——测试用另一个线程验证，
/// 跨进程语义由 `Global\` 命名对象本身保证。
pub fn acquire_single_session(wait: Duration) -> Result<Option<SingleSession>> {
    match WinMutexOps.acquire(SINGLE_SESSION_MUTEX, wait) {
        Ok(handle) => Ok(Some(SingleSession { handle })),
        Err(Error::MutexTimeout { .. }) => Ok(None),
        Err(other) => Err(other),
    }
}

#[cfg(test)]
mod single_session_tests {
    use super::*;

    #[test]
    fn second_thread_is_refused_while_the_first_holds_it() {
        let first = acquire_single_session(Duration::from_millis(0)).expect("首次获取");
        assert!(first.is_some(), "本进程应能拿到单会话锁");

        let blocked = std::thread::spawn(|| acquire_single_session(Duration::from_millis(100)));
        let result = blocked.join().expect("线程");
        assert!(
            matches!(result, Ok(None)),
            "另一个线程必须被拒，实际 {result:?}"
        );

        drop(first);
        // 释放后可以再次拿到
        let again = acquire_single_session(Duration::from_millis(100)).expect("释放后获取");
        assert!(again.is_some(), "释放后应能重新获取");
    }
}
