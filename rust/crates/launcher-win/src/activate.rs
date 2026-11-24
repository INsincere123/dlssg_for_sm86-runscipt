//! 单实例"再来一次"：把已有界面的窗口叫到最前面（常规桌面行为）。
//!
//! 界面进程启动时创建一个命名事件 `Local\DlssgLauncherUiActivate`，并起一个等待线程；
//! 后来者（第二次双击 EXE）先借一次前台权限，再 `SetEvent` 通知它。界面收到信号后走
//! **和托盘里点「显示窗口」完全相同的路径**恢复自己（清掉"交接后隐藏"、`Visible(true)`、
//! `Minimized(false)`、`Focus`），不会留下"窗口显示了、状态还以为自己在托盘里"的半截状态。
//!
//! 后来者还会按标题找一次窗口并直接 `ShowWindow` + `SetForegroundWindow`：界面此刻要是
//! 在收尾或没在处理帧，窗口也能先露出来。命名事件在 `Local\` 命名空间，只影响同一登录会话，
//! 跨用户/会话互不干扰。
//!
//! 为什么不能只靠 `SetForegroundWindow`：Windows 只允许"当前在前台的那一方"抢前台，
//! 后台进程直接调会被拒绝（窗口闪一下任务栏但不前置）。后来者是用户刚点开的进程，它调用
//! `AllowSetForegroundWindow(ASFW_ANY)` 把这次机会让给已有界面，已有界面再 `Focus` 才有权限。

use std::ffi::c_void;

use launcher_core::{Error, Result};

/// 界面单实例的激活事件名（与界面锁 `Local\DlssgLauncherUi` 分开）。
pub const ACTIVATION_EVENT: &str = "Local\\DlssgLauncherUiActivate";

const EVENT_MODIFY_STATE: u32 = 0x0002;
const WAIT_OBJECT_0: u32 = 0;
const INFINITE: u32 = 0xFFFF_FFFF;
const ASFW_ANY: u32 = 0xFFFF_FFFF;
const SW_SHOW: i32 = 5;
const SW_RESTORE: i32 = 9;

type Handle = isize;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateEventW(
        attributes: *mut c_void,
        manual_reset: i32,
        initial_state: i32,
        name: *const u16,
    ) -> Handle;
    fn OpenEventW(access: u32, inherit: i32, name: *const u16) -> Handle;
    fn SetEvent(handle: Handle) -> i32;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn CloseHandle(handle: Handle) -> i32;
}

#[link(name = "user32")]
unsafe extern "system" {
    fn AllowSetForegroundWindow(process_id: u32) -> i32;
    fn EnumWindows(callback: unsafe extern "system" fn(Handle, isize) -> i32, lparam: isize)
    -> i32;
    fn GetWindowTextLengthW(hwnd: Handle) -> i32;
    fn GetWindowTextW(hwnd: Handle, buffer: *mut u16, max_count: i32) -> i32;
    fn IsWindowVisible(hwnd: Handle) -> i32;
    fn IsIconic(hwnd: Handle) -> i32;
    fn ShowWindow(hwnd: Handle, command: i32) -> i32;
    fn SetForegroundWindow(hwnd: Handle) -> i32;
    fn BringWindowToTop(hwnd: Handle) -> i32;
    fn GetForegroundWindow() -> Handle;
    fn GetWindowThreadProcessId(hwnd: Handle, process_id: *mut u32) -> u32;
    fn AttachThreadInput(attach: u32, attach_to: u32, attach_flag: i32) -> i32;
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 界面进程持有的命名事件；起等待线程后一直活着。
pub struct ActivationEvent {
    handle: Handle,
}

impl ActivationEvent {
    /// 创建（或打开同名的）激活事件。创建失败只影响"再次运行程序"这一条路，不算致命。
    pub fn create() -> Result<Self> {
        let name = wide(ACTIVATION_EVENT);
        let handle = unsafe { CreateEventW(std::ptr::null_mut(), 0, 0, name.as_ptr()) };
        if handle == 0 {
            return Err(Error::unavailable(format!(
                "创建界面激活事件失败：{}",
                std::io::Error::last_os_error()
            )));
        }
        Ok(Self { handle })
    }

    /// 等一次信号；`timeout_ms` 为 `None` 时一直等。返回是否等到。
    pub fn wait(&self, timeout_ms: Option<u32>) -> bool {
        let timeout = timeout_ms.unwrap_or(INFINITE);
        unsafe { WaitForSingleObject(self.handle, timeout) == WAIT_OBJECT_0 }
    }
}

impl Drop for ActivationEvent {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle) };
    }
}

/// 后台等激活信号，收到就调用 `on_signal`（跑在等待线程里）。
pub fn spawn_waiter(event: ActivationEvent, on_signal: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        while event.wait(None) {
            on_signal();
        }
    });
}

/// 第二次启动：通知已有界面把窗口叫到最前面。返回是否发出了信号或找到了窗口。
pub fn request_activation(title: &str) -> bool {
    // 后来者是"用户刚点开的进程"：把这次前台机会让给已有界面，否则它的 Focus 会被系统拒绝。
    unsafe { AllowSetForegroundWindow(ASFW_ANY) };

    let mut signaled = false;
    let name = wide(ACTIVATION_EVENT);
    let handle = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
    if handle != 0 {
        signaled = unsafe { SetEvent(handle) } != 0;
        unsafe { CloseHandle(handle) };
    }
    // 界面收到信号会自己显示并聚焦；这里再直接叫一次窗口，避免它此刻没在处理帧。
    let shown = activate_window(title);
    signaled || shown
}

/// 按标题找本程序的顶层窗口：最小化就还原、隐藏就显示，然后请求前台。
pub fn activate_window(title: &str) -> bool {
    struct Search {
        title: String,
        found: Option<Handle>,
    }

    unsafe extern "system" fn callback(hwnd: Handle, lparam: isize) -> i32 {
        let search = unsafe { &mut *(lparam as *mut Search) };
        if !title_matches(hwnd, &search.title) || !owned_by_current_exe(hwnd) {
            return 1;
        }
        // 同标题只会有一个；可见的优先（隐藏的那些是别的东西）。
        if search.found.is_none() || unsafe { IsWindowVisible(hwnd) } != 0 {
            search.found = Some(hwnd);
        }
        if unsafe { IsWindowVisible(hwnd) } != 0 {
            return 0;
        }
        1
    }

    let mut search = Search {
        title: title.to_owned(),
        found: None,
    };
    unsafe { EnumWindows(callback, &mut search as *mut Search as isize) };
    let Some(hwnd) = search.found else {
        return false;
    };

    if unsafe { IsIconic(hwnd) } != 0 {
        unsafe { ShowWindow(hwnd, SW_RESTORE) };
    } else {
        unsafe { ShowWindow(hwnd, SW_SHOW) };
    }
    unsafe { BringWindowToTop(hwnd) };
    if unsafe { SetForegroundWindow(hwnd) } != 0 {
        return true;
    }
    // 系统拒绝了（前台锁）：临时借用前台线程的输入队列再试一次。
    let foreground = unsafe { GetForegroundWindow() };
    if foreground == 0 || foreground == hwnd {
        return false;
    }
    let foreground_thread = unsafe { GetWindowThreadProcessId(foreground, std::ptr::null_mut()) };
    let target_thread = unsafe { GetWindowThreadProcessId(hwnd, std::ptr::null_mut()) };
    if foreground_thread == 0 || target_thread == 0 || foreground_thread == target_thread {
        return false;
    }
    unsafe { AttachThreadInput(foreground_thread, target_thread, 1) };
    let activated = unsafe { SetForegroundWindow(hwnd) };
    unsafe { AttachThreadInput(foreground_thread, target_thread, 0) };
    activated != 0
}

fn title_matches(hwnd: Handle, wanted: &str) -> bool {
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    if length <= 0 {
        return false;
    }
    let mut buffer = vec![0u16; (length + 1) as usize];
    let copied = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    if copied <= 0 {
        return false;
    }
    String::from_utf16_lossy(&buffer[..copied as usize]) == wanted
}

/// 只认本程序自己的窗口：标题可能被别的程序撞上，进程映像能核对时必须一致。
fn owned_by_current_exe(hwnd: Handle) -> bool {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    if pid == 0 {
        return false;
    }
    let Some(own) = std::env::current_exe().ok() else {
        return true; // 拿不到自己的路径就别把窗口排除掉
    };
    let Some(actual) = crate::executable_path(pid) else {
        // 读不到对方进程路径（权限不足）：不据此否决，交给标题判断。
        return true;
    };
    actual
        .to_string_lossy()
        .trim_end_matches('\\')
        .eq_ignore_ascii_case(own.to_string_lossy().trim_end_matches('\\'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signal_reaches_the_waiter() {
        let event = ActivationEvent::create().expect("事件");
        let (sender, receiver) = std::sync::mpsc::channel();
        spawn_waiter(event, move || {
            let _ = sender.send(());
        });
        // 起线程需要一点时间；信号是自动重置事件，早发也没关系（会留在"已通知"状态）。
        std::thread::sleep(std::time::Duration::from_millis(50));
        let shipped = request_activation("DLSSG 启动器（测试：不存在的窗口标题）");
        assert!(shipped, "应当发出激活信号");
        assert!(
            receiver
                .recv_timeout(std::time::Duration::from_secs(2))
                .is_ok(),
            "等待线程必须收到激活信号"
        );
    }

    #[test]
    fn activating_a_missing_window_is_not_an_error() {
        assert!(!activate_window("DLSSG 启动器（测试：不存在的窗口标题）"));
    }
}
