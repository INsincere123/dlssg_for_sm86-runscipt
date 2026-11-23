//! 提权启动（M2-03）：`ShellExecuteExW(verb="runas")` 拉起同一 EXE 的隐藏子进程。
//!
//! 对应 PARITY 2.2：GUI 本身 `asInvoker`，只有引擎子进程提权。
//! 取消 UAC 时 `ShellExecuteExW` 返回 `ERROR_CANCELLED (1223)`，必须当作**可恢复**结果，
//! 由调用方保持界面可用并提示，而不是当异常崩掉。

use std::ffi::c_void;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::time::Duration;

use launcher_core::{Error, Result};

const SEE_MASK_NOCLOSEPROCESS: u32 = 0x0000_0040;
const SEE_MASK_NOASYNC: u32 = 0x0000_0100;
const SW_HIDE: i32 = 0;
const ERROR_CANCELLED: i32 = 1223;
const COINIT_APARTMENTTHREADED: u32 = 0x2;
const SHELL_EXECUTE_INFO_SIZE: u32 = 112;

#[repr(C)]
struct ShellExecuteInfoW {
    cb_size: u32,
    f_mask: u32,
    hwnd: isize,
    lp_verb: *const u16,
    lp_file: *const u16,
    lp_parameters: *const u16,
    lp_directory: *const u16,
    n_show: i32,
    h_inst_app: isize,
    lp_idlist: *mut c_void,
    lp_class: *const u16,
    h_key_class: isize,
    dw_hot_key: u32,
    h_icon_or_monitor: isize,
    h_process: isize,
    h_thread: isize,
}

#[link(name = "shell32")]
unsafe extern "system" {
    fn ShellExecuteExW(info: *mut ShellExecuteInfoW) -> i32;
}

#[link(name = "ole32")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, co_init: u32) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn WaitForSingleObject(handle: isize, milliseconds: u32) -> u32;
    fn GetExitCodeProcess(process: isize, code: *mut u32) -> i32;
    fn CloseHandle(handle: isize) -> i32;
}

/// 提权启动的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElevationOutcome {
    /// 子进程已运行结束，`code` 是它的退出码。
    Finished { code: u32 },
    /// 用户取消了 UAC。调用方必须保持界面可用。
    Cancelled,
}

fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

/// 把 `ShellExecuteExW` 的失败分类：1223 = 用户取消，其余是环境错误。
pub fn classify_shell_error(error: &io::Error) -> Option<ElevationOutcome> {
    match error.raw_os_error() {
        Some(ERROR_CANCELLED) => Some(ElevationOutcome::Cancelled),
        _ => None,
    }
}

/// 以管理员身份运行同一台机器上的一个 EXE，隐藏窗口并等待它结束。
pub fn run_elevated(exe: &Path, arguments: &str, wait: Duration) -> Result<ElevationOutcome> {
    // ShellExecuteEx 需要 COM 初始化；重复初始化返回 S_FALSE 也无害。
    unsafe { CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED) };

    let verb = wide(std::ffi::OsStr::new("runas"));
    let file = wide(exe.as_os_str());
    let parameters = wide(std::ffi::OsStr::new(arguments));
    let mut info: ShellExecuteInfoW = unsafe { std::mem::zeroed() };
    // 必须传 112：x64 的 sizeof(SHELLEXECUTEINFOW) 是 112，而本结构体按字段对齐算出来是 120。
    // 传 120 会被 Windows 以 ERROR_ACCESS_DENIED(5) 拒绝（实测 2026-11-22，见 M2-03 排查记录）。
    info.cb_size = SHELL_EXECUTE_INFO_SIZE;
    info.f_mask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    info.lp_verb = verb.as_ptr();
    info.lp_file = file.as_ptr();
    info.lp_parameters = parameters.as_ptr();
    info.n_show = SW_HIDE;

    let started = unsafe { ShellExecuteExW(&mut info) };
    if started == 0 {
        let error = io::Error::last_os_error();
        if let Some(outcome) = classify_shell_error(&error) {
            return Ok(outcome);
        }
        return Err(Error::io(exe, error));
    }
    if info.h_process == 0 {
        return Err(Error::unavailable(
            "提权启动成功但拿不到进程句柄，无法等待结果。",
        ));
    }
    let milliseconds = u32::try_from(wait.as_millis()).unwrap_or(u32::MAX);
    unsafe { WaitForSingleObject(info.h_process, milliseconds) };
    let mut code: u32 = 0;
    let read = unsafe { GetExitCodeProcess(info.h_process, &mut code) };
    unsafe { CloseHandle(info.h_process) };
    if read == 0 {
        return Err(last_error(exe));
    }
    Ok(ElevationOutcome::Finished { code })
}

/// 提权启动后**不等待**：界面把工作交给它就可以退出。
///
/// 与 `run_elevated` 的区别是不读退出码、不阻塞；句柄立刻关闭不会影响子进程。
pub fn spawn_elevated(exe: &Path, arguments: &str) -> Result<()> {
    unsafe { CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED) };

    let verb = wide(std::ffi::OsStr::new("runas"));
    let file = wide(exe.as_os_str());
    let parameters = wide(std::ffi::OsStr::new(arguments));
    let mut info: ShellExecuteInfoW = unsafe { std::mem::zeroed() };
    info.cb_size = SHELL_EXECUTE_INFO_SIZE;
    info.f_mask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    info.lp_verb = verb.as_ptr();
    info.lp_file = file.as_ptr();
    info.lp_parameters = parameters.as_ptr();
    info.n_show = SW_HIDE;

    let started = unsafe { ShellExecuteExW(&mut info) };
    if started == 0 {
        let error = io::Error::last_os_error();
        if let Some(outcome) = classify_shell_error(&error) {
            return Err(Error::unavailable(match outcome {
                ElevationOutcome::Cancelled => "用户取消了提权".to_owned(),
                _ => "提权启动失败".to_owned(),
            }));
        }
        return Err(Error::io(exe, error));
    }
    if info.h_process != 0 {
        unsafe { CloseHandle(info.h_process) };
    }
    Ok(())
}
fn last_error(path: &Path) -> Error {
    Error::io(path, io::Error::last_os_error())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_is_classified_as_cancelled() {
        let error = io::Error::from_raw_os_error(ERROR_CANCELLED);
        assert_eq!(
            classify_shell_error(&error),
            Some(ElevationOutcome::Cancelled)
        );
    }

    #[test]
    fn other_errors_are_not_cancellation() {
        for code in [2, 5, 87, 193] {
            let error = io::Error::from_raw_os_error(code);
            assert_eq!(classify_shell_error(&error), None, "错误码 {code}");
        }
    }

    #[test]
    fn wide_conversion_appends_nul() {
        let value = wide(std::ffi::OsStr::new("D:\\a.exe"));
        assert_eq!(value.last(), Some(&0u16));
        assert_eq!(value.len(), "D:\\a.exe".len() + 1);
    }
}
