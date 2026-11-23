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
pub mod gpu;
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
    fn ShellExecuteW(
        window: Handle,
        operation: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show: i32,
    ) -> Handle;
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
    fn CreateProcessW(
        application: *const u16,
        command_line: *mut u16,
        process_attributes: *const c_void,
        thread_attributes: *const c_void,
        inherit: i32,
        flags: u32,
        environment: *mut c_void,
        directory: *const u16,
        startup: *mut StartupInfo,
        process: *mut ProcessInformation,
    ) -> i32;
    fn GetExitCodeProcess(process: Handle, code: *mut u32) -> i32;
    fn OpenMutexW(access: u32, inherit: i32, name: *const u16) -> Handle;
    fn TerminateProcess(process: Handle, code: u32) -> i32;
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

// ---------------------------------------------------------------- 启动子进程

#[repr(C)]
struct StartupInfo {
    size: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    x: u32,
    y: u32,
    x_size: u32,
    y_size: u32,
    x_chars: u32,
    y_chars: u32,
    fill: u32,
    flags: u32,
    show: u16,
    reserved2: u16,
    reserved3: *mut u8,
    stdin: Handle,
    stdout: Handle,
    stderr: Handle,
}

#[repr(C)]
struct ProcessInformation {
    process: Handle,
    thread: Handle,
    process_id: u32,
    thread_id: u32,
}

/// 启动子进程的结果。
#[derive(Debug)]
pub struct Spawned {
    pub pid: u32,
    handle: Handle,
}

impl Spawned {
    /// 等它结束并取退出码（`None` 表示超时）。
    pub fn wait(&self, timeout: std::time::Duration) -> Option<u32> {
        const WAIT_OBJECT_0: u32 = 0;
        let milliseconds = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        let waited = unsafe { WaitForSingleObject(self.handle, milliseconds) };
        if waited != WAIT_OBJECT_0 {
            return None;
        }
        let mut code: u32 = 0;
        if unsafe { GetExitCodeProcess(self.handle, &mut code) } == 0 {
            return None;
        }
        Some(code)
    }

    /// 放弃等待（不终止子进程）；句柄由 `Drop` 关闭。
    pub fn forget(mut self) {
        self.handle = 0;
    }
}

impl Drop for Spawned {
    fn drop(&mut self) {
        if self.handle != 0 && self.handle != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(self.handle) };
            self.handle = 0;
        }
    }
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
        let first = unsafe {
            if info.target_is_dir {
                RemoveDirectoryW(wide.as_ptr())
            } else {
                DeleteFileW(wide.as_ptr())
            }
        };
        if first != 0 {
            return Ok(());
        }
        // 悬空的目录符号链接在部分 Windows 版本上 RemoveDirectory 会返回"拒绝访问"，
        // 换 DeleteFile 再试一次（实测 2026-11-23 于异环资源链接上）。
        let error = std::io::Error::last_os_error();
        let second = unsafe {
            if info.target_is_dir {
                DeleteFileW(wide.as_ptr())
            } else {
                RemoveDirectoryW(wide.as_ptr())
            }
        };
        if second == 0 {
            return Err(Error::io(path, error));
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

/// 一组"要匹配的进程"条件：完整路径、进程名（不带 `.exe`）、所在目录，三者任一命中即可。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProcessFilter {
    pub executables: Vec<PathBuf>,
    pub names: Vec<String>,
    pub directory: Option<PathBuf>,
}

impl ProcessFilter {
    pub fn is_empty(&self) -> bool {
        self.executables.is_empty() && self.names.is_empty() && self.directory.is_none()
    }

    fn matches(&self, entry: &launcher_core::ProcessEntry) -> bool {
        let path = entry
            .executable
            .as_ref()
            .map(|path| path.to_string_lossy().replace('/', "\\").to_lowercase());
        if let Some(path) = path.as_deref() {
            let by_path = self.executables.iter().any(|candidate| {
                candidate
                    .to_string_lossy()
                    .replace('/', "\\")
                    .to_lowercase()
                    == path
            });
            if by_path {
                return true;
            }
            if let Some(directory) = self.directory.as_ref() {
                let prefix = format!(
                    "{}\\",
                    directory
                        .to_string_lossy()
                        .replace('/', "\\")
                        .trim_end_matches('\\')
                        .to_lowercase()
                );
                if path.starts_with(&prefix) {
                    return true;
                }
            }
        }
        // 路径读不到（提权进程）时按名字兜底。
        self.names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&entry.name))
    }
}
/// 找出与给定可执行文件匹配的进程（完整路径或进程名，大小写不敏感）。
///
/// 游戏常常以管理员身份运行，非提权进程的 `QueryFullProcessImageName` 会失败，
/// 所以路径读不到时退一步用进程名匹配——这正是"检测到游戏在跑"需要的鲁棒性。
pub fn processes_matching(executables: &[PathBuf]) -> Vec<u32> {
    let filter = ProcessFilter {
        executables: executables.to_vec(),
        names: executables
            .iter()
            .filter_map(|path| {
                path.file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
            })
            .collect(),
        directory: None,
    };
    matching_processes(&filter)
}

/// 命中过滤条件的进程号（不含本进程）。
pub fn matching_processes(filter: &ProcessFilter) -> Vec<u32> {
    if filter.is_empty() {
        return Vec::new();
    }
    let entries = match WinProcessProbe.list() {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };
    let current = unsafe { GetCurrentProcessId() };
    entries
        .into_iter()
        .filter(|entry| entry.pid != current)
        .filter(|entry| filter.matches(entry))
        .map(|entry| entry.pid)
        .collect()
}

/// 结束命中过滤条件的所有进程，并等它们退出（最多 `wait`）。
///
/// 返回 `(已结束的 PID, 失败的 PID)`；游戏以管理员运行时调用方必须提权。
pub fn terminate_matching(
    filter: &ProcessFilter,
    wait: std::time::Duration,
) -> (Vec<u32>, Vec<u32>) {
    let mut terminated: Vec<u32> = Vec::new();
    let mut failed: Vec<u32> = Vec::new();
    for pid in matching_processes(filter) {
        if terminate_pid(pid, wait) {
            terminated.push(pid);
        } else {
            failed.push(pid);
        }
    }
    (terminated, failed)
}

/// 找出可执行文件位于给定目录之内的进程（大小写不敏感，要求 `\` 边界）。
///
/// 用于"官方启动器已经起来了吗"这类判断：二游的启动器进程名常常和入口 EXE 不同
/// （鸣潮入口 `NTELauncher.exe` 起的是 `NTELauncher\NTEGame.exe`），按目录判断更稳。
pub fn processes_in_directory(directory: &Path) -> Vec<u32> {
    let root = directory
        .to_string_lossy()
        .replace('/', "\\")
        .to_lowercase();
    let prefix = format!("{}\\", root.trim_end_matches('\\'));
    let entries = match WinProcessProbe.list() {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };
    let current = unsafe { GetCurrentProcessId() };
    entries
        .into_iter()
        .filter(|entry| entry.pid != current)
        .filter(|entry| {
            entry
                .executable
                .as_ref()
                .map(|path| {
                    path.to_string_lossy()
                        .replace('/', "\\")
                        .to_lowercase()
                        .starts_with(&prefix)
                })
                .unwrap_or(false)
        })
        .map(|entry| entry.pid)
        .collect()
}

/// 指定 PID 的进程是否还活着（打不开也算不存在：可能已退出或权限不足）。
pub fn process_running(pid: u32) -> bool {
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;
    if pid == 0 {
        return false;
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle == 0 {
        return false;
    }
    let mut code: u32 = 0;
    let ok = unsafe { GetExitCodeProcess(handle, &mut code) };
    unsafe { CloseHandle(handle) };
    ok != 0 && code == STILL_ACTIVE
}

/// 结束指定 PID 的进程（需要权限），并等它退出。
pub fn terminate_pid(pid: u32, wait: std::time::Duration) -> bool {
    const PROCESS_TERMINATE: u32 = 0x0001;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    if pid == 0 {
        return false;
    }
    let handle = unsafe {
        OpenProcess(
            PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            pid,
        )
    };
    if handle == 0 {
        return false;
    }
    let ok = unsafe { TerminateProcess(handle, 1) };
    unsafe { WaitForSingleObject(handle, wait.as_millis().min(30_000) as u32) };
    unsafe { CloseHandle(handle) };
    ok != 0
}

/// 结束与给定可执行文件匹配的所有进程，并等它们退出（最多 `wait`）。
///
/// 返回 `(已结束的 PID, 失败的 PID)`；需要权限（游戏以管理员运行时，调用方必须提权）。
pub fn terminate_processes(
    executables: &[PathBuf],
    wait: std::time::Duration,
) -> (Vec<u32>, Vec<u32>) {
    const PROCESS_TERMINATE: u32 = 0x0001;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

    let mut terminated: Vec<u32> = Vec::new();
    let mut failed: Vec<u32> = Vec::new();
    for pid in processes_matching(executables) {
        let handle = unsafe {
            OpenProcess(
                PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                pid,
            )
        };
        if handle == 0 {
            failed.push(pid);
            continue;
        }
        let ok = unsafe { TerminateProcess(handle, 1) };
        // 等它真的退出，避免后续恢复步骤还看到残留文件被占用。
        unsafe { WaitForSingleObject(handle, wait.as_millis().min(30_000) as u32) };
        unsafe { CloseHandle(handle) };
        if ok != 0 {
            terminated.push(pid);
        } else {
            failed.push(pid);
        }
    }
    (terminated, failed)
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

/// 启动一个子进程（不经过 shell）。`hidden` 为真时不显示窗口。
///
/// `arguments` 按 Windows 命令行规则直接拼在可执行文件后面；路径由调用方保证已加引号。
pub fn spawn_process(
    executable: &Path,
    arguments: &[String],
    working_directory: Option<&Path>,
    hidden: bool,
) -> Result<Spawned> {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let application = path_wide(executable);
    let mut command_line: Vec<u16> = Vec::new();
    for unit in std::ffi::OsStr::new(&format!("\"{}\"", executable.display())).encode_wide() {
        command_line.push(unit);
    }
    for argument in arguments {
        command_line.push(b' ' as u16);
        for unit in std::ffi::OsStr::new(argument).encode_wide() {
            command_line.push(unit);
        }
    }
    command_line.push(0);
    let directory = working_directory.map(path_wide);

    let mut startup: StartupInfo = unsafe { std::mem::zeroed() };
    startup.size = std::mem::size_of::<StartupInfo>() as u32;
    let mut process: ProcessInformation = unsafe { std::mem::zeroed() };
    let flags = if hidden { CREATE_NO_WINDOW } else { 0 };
    let started = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            flags,
            std::ptr::null_mut(),
            directory
                .as_ref()
                .map_or(std::ptr::null(), |value| value.as_ptr()),
            &mut startup,
            &mut process,
        )
    };
    if started == 0 {
        return Err(Error::io(executable, std::io::Error::last_os_error()));
    }
    unsafe { CloseHandle(process.thread) };
    Ok(Spawned {
        pid: process.process_id,
        handle: process.process,
    })
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

/// 用系统默认程序打开一个路径（日志目录、偏好文件等）。
pub fn open_path(path: &Path) -> Result<()> {
    const SW_SHOWNORMAL: i32 = 1;
    let operation = wide_str("open");
    let file = path_wide(path);
    let result = unsafe {
        ShellExecuteW(
            0,
            operation.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW 返回值 > 32 表示成功。
    if result <= 32 {
        return Err(Error::io_plain(std::io::Error::last_os_error()));
    }
    Ok(())
}

/// 只探测"引擎会话是否在跑"：不抢占锁，拿不到就是没有。
pub fn session_active() -> bool {
    const SYNCHRONIZE: u32 = 0x0010_0000;
    let name = wide_str(SINGLE_SESSION_MUTEX);
    let handle = unsafe { OpenMutexW(SYNCHRONIZE, 0, name.as_ptr()) };
    if handle == 0 || handle == INVALID_HANDLE_VALUE {
        return false;
    }
    unsafe { CloseHandle(handle) };
    true
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
    acquire_named_session(SINGLE_SESSION_MUTEX, wait)
}

/// 任意具名互斥的单实例锁（界面用 `Local\DlssgLauncherUi`，与引擎会话锁分开）。
pub fn acquire_named_session(name: &str, wait: Duration) -> Result<Option<SingleSession>> {
    match WinMutexOps.acquire(name, wait) {
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
