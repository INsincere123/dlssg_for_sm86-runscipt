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

pub mod activate;
pub mod drives;
pub mod elevate;
pub mod fontmap;
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
const ERROR_NO_MORE_FILES: u32 = 18;
const ERROR_INVALID_PARAMETER: i32 = 87;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

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
    fn GetCurrentProcess() -> Handle;
    fn DuplicateHandle(
        source_process: Handle,
        source: Handle,
        target_process: Handle,
        target: *mut Handle,
        access: u32,
        inherit: i32,
        options: u32,
    ) -> i32;
    fn GetProcessId(process: Handle) -> u32;
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

#[link(name = "advapi32")]
unsafe extern "system" {
    fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
    fn GetTokenInformation(
        token: Handle,
        class: u32,
        info: *mut c_void,
        length: u32,
        needed: *mut u32,
    ) -> i32;
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
    /// 将等待句柄交给同一 EXE 的另一个进程。交接失败时收回复制的句柄。
    pub fn transfer_to(&self, pid: u32, publish: impl FnOnce(isize) -> Result<()>) -> Result<()> {
        const PROCESS_DUP_HANDLE: u32 = 0x40;
        const DUPLICATE_CLOSE_SOURCE: u32 = 0x1;
        const DUPLICATE_SAME_ACCESS: u32 = 0x2;
        let own = std::env::current_exe().map_err(Error::io_plain)?;
        let target = unsafe {
            OpenProcess(
                PROCESS_DUP_HANDLE | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                pid,
            )
        };
        if target == 0 {
            return Err(last_error());
        }
        let result = (|| {
            if !executable_path_of(target).is_some_and(|path| {
                path.to_string_lossy()
                    .eq_ignore_ascii_case(&own.to_string_lossy())
            }) {
                return Err(Error::invalid("无法核对托盘进程的程序路径。"));
            }
            let current = unsafe { GetCurrentProcess() };
            let mut remote = 0;
            if unsafe {
                DuplicateHandle(
                    current,
                    self.handle,
                    target,
                    &mut remote,
                    0,
                    0,
                    DUPLICATE_SAME_ACCESS,
                )
            } == 0
            {
                return Err(last_error());
            }
            let result = publish(remote);
            if result.is_err() {
                // DUPLICATE_CLOSE_SOURCE 即使复制失败也会关闭源句柄。
                unsafe {
                    DuplicateHandle(
                        target,
                        remote,
                        0,
                        std::ptr::null_mut(),
                        0,
                        0,
                        DUPLICATE_CLOSE_SOURCE,
                    )
                };
            }
            result
        })();
        unsafe { CloseHandle(target) };
        result
    }

    /// # Safety
    /// `handle` 必须是已转交给本进程且尚未被接管的进程句柄。
    pub unsafe fn from_transferred_handle(handle: isize) -> Self {
        Self {
            pid: unsafe { GetProcessId(handle) },
            handle,
        }
    }

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

    /// 查询子进程是否已退出。句柄查询失败与子进程仍在运行是不同结果。
    pub fn try_wait(&self) -> Result<Option<u32>> {
        match unsafe { WaitForSingleObject(self.handle, 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut code = 0;
                if unsafe { GetExitCodeProcess(self.handle, &mut code) } == 0 {
                    return Err(last_error());
                }
                Ok(Some(code))
            }
            _ => Err(last_error()),
        }
    }

    /// 放弃等待（仅关闭本进程持有的句柄）。
    pub fn forget(self) {
        drop(self);
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
        if ok == 0 {
            let error = last_error();
            unsafe { CloseHandle(snapshot) };
            return Err(error);
        }
        while ok != 0 {
            entries.push(ProcessEntry {
                pid: raw.process_id,
                name: process_name(&raw),
                executable: executable_path(raw.process_id),
                started_at: started_at(raw.process_id).ok(),
            });
            ok = unsafe { Process32NextW(snapshot, &mut raw) };
        }
        let code = last_error_code();
        unsafe { CloseHandle(snapshot) };
        if code != ERROR_NO_MORE_FILES {
            return Err(Error::io_plain(std::io::Error::from_raw_os_error(
                code as i32,
            )));
        }
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

/// 查询已知 PID 是否存活。
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

fn process_name(entry: &ProcessEntry32W) -> String {
    let end = entry
        .exe_file
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(entry.exe_file.len());
    let text = String::from_utf16_lossy(&entry.exe_file[..end]);
    Path::new(&text)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(&text)
        .to_owned()
}

pub(crate) fn executable_path(pid: u32) -> Option<PathBuf> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle == 0 {
        return None;
    }
    let path = executable_path_of(handle);
    unsafe { CloseHandle(handle) };
    path
}

fn executable_path_of(handle: Handle) -> Option<PathBuf> {
    let mut buffer = [0u16; 1024];
    query_executable_path(handle, &mut buffer).or_else(|| {
        (last_error_code() == ERROR_INSUFFICIENT_BUFFER)
            .then(|| query_executable_path(handle, &mut vec![0u16; 32768]))
            .flatten()
    })
}

fn query_executable_path(handle: Handle, buffer: &mut [u16]) -> Option<PathBuf> {
    let mut size = buffer.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut size) };
    if ok == 0 {
        return None;
    }
    Some(PathBuf::from(String::from_utf16_lossy(
        &buffer[..size as usize],
    )))
}

/// 对同一个打开的句柄核对完整路径并结束进程，避免 PID 回收后误伤其他进程。
pub fn terminate_verified(
    pid: u32,
    targets: &launcher_core::session::ExecutableTargets,
    wait: std::time::Duration,
) -> Result<()> {
    const PROCESS_TERMINATE: u32 = 0x0001;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    if pid == std::process::id() {
        return Err(Error::invalid("不能结束当前恢复进程。"));
    }
    let handle = unsafe {
        OpenProcess(
            PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE,
            0,
            pid,
        )
    };
    if handle == 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER) {
            return Ok(());
        }
        return Err(Error::io_plain(error));
    }
    let result = (|| {
        if unsafe { WaitForSingleObject(handle, 0) } == WAIT_OBJECT_0 {
            return Ok(());
        }
        let executable = executable_path_of(handle)
            .ok_or_else(|| Error::unavailable(format!("无法在结束前核验进程 PID {pid}")))?;
        if !targets.matches(&launcher_core::path::PathContext::current()?, &executable)? {
            return Err(Error::invalid(format!(
                "PID {pid} 的路径已变化：{}",
                executable.display()
            )));
        }
        if unsafe { TerminateProcess(handle, 1) } == 0 {
            return Err(last_error());
        }
        if unsafe { WaitForSingleObject(handle, wait.as_millis().min(30_000) as u32) }
            != WAIT_OBJECT_0
        {
            return Err(Error::unavailable(format!("等待进程 PID {pid} 退出超时")));
        }
        Ok(())
    })();
    unsafe { CloseHandle(handle) };
    result
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
///
/// 用令牌里的 `TokenElevation` 判断，不启动子进程：以前跑 `net session`，
/// 在 `windows` 子系统的提权子进程里会闪一下控制台窗口
/// （显卡改名正是跑在那条路径上），而且 Server 服务停掉时会误报未提权。
pub fn is_elevated() -> bool {
    const TOKEN_QUERY: u32 = 0x0008;
    const TOKEN_ELEVATION: u32 = 20;
    let mut token: Handle = 0;
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return false;
    }
    let mut elevated: u32 = 0;
    let mut needed: u32 = 0;
    let ok = unsafe {
        GetTokenInformation(
            token,
            TOKEN_ELEVATION,
            (&mut elevated as *mut u32).cast::<c_void>(),
            4,
            &mut needed,
        )
    };
    unsafe { CloseHandle(token) };
    ok != 0 && elevated != 0
}

/// 用系统默认程序打开一个路径（日志目录、偏好文件等）。
pub fn open_path(path: &Path) -> Result<()> {
    let file = path_wide(path);
    shell_open(&file, &path.display().to_string())
}

/// 用系统默认处理器打开一个目标：协议 URI。
///
/// Epic 游戏靠它走 `com.epicgames.launcher://apps/<AppName>?action=launch&silent=true`：
/// 由系统找到（必要时拉起）Epic 启动器，再让它启动游戏。直接起渲染 EXE 会绕开
/// Epic 的归属校验和在线服务，和 Steam 那边是同一类问题。
pub fn open_target(target: &str) -> Result<()> {
    if target.trim().is_empty() {
        return Err(Error::invalid("要打开的目标为空。"));
    }
    shell_open(&wide_str(target), target)
}

fn shell_open(file: &[u16], label: &str) -> Result<()> {
    const SW_SHOWNORMAL: i32 = 1;
    let operation = wide_str("open");
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
    // ShellExecuteW 返回值 > 32 表示成功；≤ 32 是它自己的错误码（不是 LastError）。
    if result <= 32 {
        return Err(Error::unavailable(format!(
            "系统没能打开 {label}（ShellExecuteW 返回 {result}）：缺少对应的启动器或协议处理器？"
        )));
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
