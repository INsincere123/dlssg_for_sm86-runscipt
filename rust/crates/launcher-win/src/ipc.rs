//! 引擎与界面之间的状态通道（M2-04）。
//!
//! 两条通道，优先级从高到低：
//! 1. **命名管道** `\\.\pipe\DlssgLauncher-<gameId>`：管道服务端带**显式 DACL**
//!    （SYSTEM / Administrators / 当前用户 完全控制），只有同机同用户能读。
//! 2. **`status.json`**：管道断开、界面后启动、或另一个进程在写时，读这个文件拿到同一阶段。
//!
//! 约定：引擎**每次都写 `status.json`**，管道只作为实时推送；因此"管道不可用时读到同一阶段"
//! 是结构保证，不是尽力而为。

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use launcher_core::{Error, Result};

type Handle = isize;
const INVALID_HANDLE_VALUE: Handle = -1;
const PIPE_ACCESS_OUTBOUND: u32 = 0x0000_0002;
const PIPE_TYPE_BYTE: u32 = 0x0000_0000;
const PIPE_READMODE_BYTE: u32 = 0x0000_0000;
const PIPE_WAIT: u32 = 0x0000_0000;
const GENERIC_READ_ACCESS: u32 = 0x8000_0000;
const OPEN_EXISTING: u32 = 3;
const SDDL_REVISION_1: u32 = 1;
const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_USER_CLASS: u32 = 1;

#[repr(C)]
struct SecurityAttributes {
    length: u32,
    descriptor: *mut std::ffi::c_void,
    inherit: i32,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        max_instances: u32,
        out_size: u32,
        in_size: u32,
        timeout: u32,
        attributes: *const SecurityAttributes,
    ) -> Handle;
    fn ConnectNamedPipe(pipe: Handle, overlapped: *mut std::ffi::c_void) -> i32;
    fn DisconnectNamedPipe(pipe: Handle) -> i32;
    fn WriteFile(
        file: Handle,
        buffer: *const u8,
        size: u32,
        written: *mut u32,
        overlapped: *mut std::ffi::c_void,
    ) -> i32;
    fn ReadFile(
        file: Handle,
        buffer: *mut u8,
        size: u32,
        read: *mut u32,
        overlapped: *mut std::ffi::c_void,
    ) -> i32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        attributes: *const SecurityAttributes,
        disposition: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    fn CloseHandle(handle: Handle) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess() -> Handle;
    fn LocalFree(block: isize) -> isize;
}

#[link(name = "advapi32")]
unsafe extern "system" {
    fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
    fn GetTokenInformation(
        token: Handle,
        class: u32,
        information: *mut std::ffi::c_void,
        length: u32,
        returned: *mut u32,
    ) -> i32;
    fn ConvertSidToStringSidW(sid: *mut std::ffi::c_void, string: *mut *mut u16) -> i32;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        sddl: *const u16,
        revision: u32,
        descriptor: *mut *mut std::ffi::c_void,
        size: *mut u32,
    ) -> i32;
}

/// 管道路径。
pub fn pipe_name(game_id: &str) -> String {
    format!("\\\\.\\pipe\\DlssgLauncher-{game_id}")
}

/// 引擎对外发布的一条状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusSnapshot {
    pub game_id: String,
    pub phase: String,
    pub pid: u32,
    /// UNIX 秒；`status.json` 与界面都只看秒级精度。
    pub updated_at: u64,
    pub message: String,
}

impl StatusSnapshot {
    pub fn new(game_id: &str, phase: &str, message: &str) -> Self {
        Self {
            game_id: game_id.to_owned(),
            phase: phase.to_owned(),
            pid: std::process::id(),
            updated_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|value| value.as_secs())
                .unwrap_or(0),
            message: message.to_owned(),
        }
    }

    /// 固定字段顺序的单行 JSON（管道一行一条，文件则是同一份内容）。
    pub fn to_json(&self) -> String {
        format!(
            "{{\"game_id\":\"{}\",\"phase\":\"{}\",\"pid\":{},\"updated_at\":{},\"message\":\"{}\"}}",
            escape(&self.game_id),
            escape(&self.phase),
            self.pid,
            self.updated_at,
            escape(&self.message)
        )
    }

    /// 只解析本模块写出的固定格式；字段缺失返回 `None`，不猜。
    pub fn from_json(text: &str) -> Option<Self> {
        let field = |name: &str| read_json_string(text, name);
        let number = |name: &str| -> Option<u64> {
            let key = format!("\"{name}\":");
            let start = text.find(&key)? + key.len();
            let digits: String = text[start..]
                .chars()
                .take_while(|character| character.is_ascii_digit())
                .collect();
            digits.parse().ok()
        };
        Some(Self {
            game_id: field("game_id")?,
            phase: field("phase")?,
            pid: number("pid")? as u32,
            updated_at: number("updated_at")?,
            message: field("message")?,
        })
    }
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

/// 读一个字符串字段：按转义规则扫描到真正的结束引号。
fn read_json_string(text: &str, name: &str) -> Option<String> {
    let marker = format!("\"{name}\":\"");
    let start = text.find(&marker)? + marker.len();
    let characters: Vec<char> = text[start..].chars().collect();
    let mut value = String::new();
    let mut index = 0;
    while index < characters.len() {
        let current = characters[index];
        if current == '\\' {
            let next = *characters.get(index + 1)?;
            value.push(match next {
                'n' => '\n',
                't' => '\t',
                '"' => '"',
                '\\' => '\\',
                other => other,
            });
            index += 2;
            continue;
        }
        if current == '"' {
            return Some(value);
        }
        value.push(current);
        index += 1;
    }
    None
}

/// `status.json` 的默认位置：`<数据目录>/status.json`。
pub fn status_path(data_directory: &Path) -> PathBuf {
    data_directory.join("status.json")
}

/// 原子写 `status.json`（同目录临时文件 + rename）。
pub fn write_status(path: &Path, status: &StatusSnapshot) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| Error::io(parent, error))?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, status.to_json()).map_err(|error| Error::io(&temporary, error))?;
    let outcome = std::fs::rename(&temporary, path).map_err(|error| Error::io(path, error));
    if temporary.exists() {
        let _ = std::fs::remove_file(&temporary);
    }
    outcome
}

/// 读 `status.json`；不存在或格式不认识都返回 `None`。
pub fn read_status(path: &Path) -> Option<StatusSnapshot> {
    let text = std::fs::read_to_string(path).ok()?;
    StatusSnapshot::from_json(text.trim())
}

/// 管道服务端。一次会话一个实例名。
pub struct PipeServer {
    handle: Handle,
    name: String,
    connected: bool,
}

impl PipeServer {
    /// 建管道并立刻等待连接（非阻塞式：先建好，`write_line` 前再 connect）。
    pub fn create(game_id: &str) -> Result<Self> {
        let name = pipe_name(game_id);
        let wide: Vec<u16> = std::ffi::OsStr::new(&name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let descriptor = security_descriptor()?;
        let attributes = SecurityAttributes {
            length: std::mem::size_of::<SecurityAttributes>() as u32,
            descriptor,
            inherit: 0,
        };
        let handle = unsafe {
            CreateNamedPipeW(
                wide.as_ptr(),
                PIPE_ACCESS_OUTBOUND,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                1,
                4096,
                4096,
                0,
                &attributes,
            )
        };
        unsafe { LocalFree(descriptor as isize) };
        if handle == INVALID_HANDLE_VALUE {
            return Err(Error::io_plain(io::Error::last_os_error()));
        }
        Ok(Self {
            handle,
            name,
            connected: false,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// 写一行（自动补 `\n`）。对端未连接时返回错误，由调用方退回 `status.json`。
    pub fn write_line(&mut self, line: &str) -> Result<()> {
        if !self.connected {
            // ERROR_PIPE_CONNECTED(535) 表示对端已经连上；其余情况先等一次。
            let connected = unsafe { ConnectNamedPipe(self.handle, std::ptr::null_mut()) };
            if connected == 0 {
                let code = io::Error::last_os_error().raw_os_error().unwrap_or(0);
                if code != 535 {
                    return Err(Error::io_plain(io::Error::last_os_error()));
                }
            }
            self.connected = true;
        }
        let mut payload = line.as_bytes().to_vec();
        payload.push(b'\n');
        let mut written: u32 = 0;
        let ok = unsafe {
            WriteFile(
                self.handle,
                payload.as_ptr(),
                payload.len() as u32,
                &mut written,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(Error::io_plain(io::Error::last_os_error()));
        }
        Ok(())
    }
}

impl Drop for PipeServer {
    fn drop(&mut self) {
        if self.connected {
            unsafe { DisconnectNamedPipe(self.handle) };
        }
        unsafe { CloseHandle(self.handle) };
    }
}

/// 管道客户端：读一行（供界面使用；M4 会接上）。
pub fn read_line(game_id: &str) -> Result<String> {
    let name = pipe_name(game_id);
    let wide: Vec<u16> = std::ffi::OsStr::new(&name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ_ACCESS,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            0,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(Error::io_plain(io::Error::last_os_error()));
    }
    let mut buffer = vec![0u8; 4096];
    let mut read: u32 = 0;
    let ok = unsafe {
        ReadFile(
            handle,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            &mut read,
            std::ptr::null_mut(),
        )
    };
    unsafe { CloseHandle(handle) };
    if ok == 0 {
        return Err(Error::io_plain(io::Error::last_os_error()));
    }
    Ok(String::from_utf8_lossy(&buffer[..read as usize])
        .trim_end()
        .to_owned())
}

/// 发布状态：**永远**写 `status.json`，再尽力推管道。返回是否推到了管道。
pub fn publish(
    server: Option<&mut PipeServer>,
    path: &Path,
    status: &StatusSnapshot,
) -> Result<bool> {
    write_status(path, status)?;
    if let Some(server) = server {
        return Ok(server.write_line(&status.to_json()).is_ok());
    }
    Ok(false)
}

/// 构造只允许 SYSTEM / Administrators / 当前用户访问的管道安全描述符。
fn security_descriptor() -> Result<*mut std::ffi::c_void> {
    let sid = current_user_sid_string()?;
    // D: 自主访问控制；A;;GA;;; 允许完全控制；SY/BA 是系统与管理员组。
    let sddl = format!("D:(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{sid})");
    let wide: Vec<u16> = std::ffi::OsStr::new(&sddl)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut descriptor: *mut std::ffi::c_void = std::ptr::null_mut();
    let mut size: u32 = 0;
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            &mut size,
        )
    };
    if ok == 0 || descriptor.is_null() {
        return Err(Error::io_plain(io::Error::last_os_error()));
    }
    Ok(descriptor)
}

fn current_user_sid_string() -> Result<String> {
    let mut token: Handle = 0;
    let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
    if ok == 0 {
        return Err(Error::io_plain(io::Error::last_os_error()));
    }
    let mut buffer = vec![0u8; 256];
    let mut returned: u32 = 0;
    let queried = unsafe {
        GetTokenInformation(
            token,
            TOKEN_USER_CLASS,
            buffer.as_mut_ptr() as *mut std::ffi::c_void,
            buffer.len() as u32,
            &mut returned,
        )
    };
    unsafe { CloseHandle(token) };
    if queried == 0 {
        return Err(Error::io_plain(io::Error::last_os_error()));
    }
    // TOKEN_USER 的第一个字段就是 PSID。
    let sid = unsafe { *(buffer.as_ptr() as *const *mut std::ffi::c_void) };
    let mut text: *mut u16 = std::ptr::null_mut();
    let converted = unsafe { ConvertSidToStringSidW(sid, &mut text) };
    if converted == 0 || text.is_null() {
        return Err(Error::io_plain(io::Error::last_os_error()));
    }
    let length = unsafe {
        (0..)
            .map(|index| *text.add(index))
            .position(|unit| unit == 0)
            .unwrap_or(0)
    };
    let units = unsafe { std::slice::from_raw_parts(text, length) };
    let result = String::from_utf16_lossy(units);
    unsafe { LocalFree(text as isize) };
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trips_and_escapes() {
        let status = StatusSnapshot {
            game_id: "steam-4356430".to_owned(),
            phase: "deploying".to_owned(),
            pid: 42,
            updated_at: 1_795_355_769,
            message: "路径 D:\\Temp\\a\"b".to_owned(),
        };
        let text = status.to_json();
        assert!(!text.contains('\n'));
        assert_eq!(StatusSnapshot::from_json(&text), Some(status));
    }

    #[test]
    fn status_file_is_written_atomically_and_read_back() {
        let directory =
            std::env::temp_dir().join(format!("dlssg-ipc-{}-write", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let path = status_path(&directory);
        let status = StatusSnapshot::new("fixture", "deploying", "开始");
        write_status(&path, &status).expect("写");
        assert_eq!(read_status(&path), Some(status));
        assert!(
            !path.with_extension("json.tmp").exists(),
            "不应残留临时文件"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn missing_status_file_reads_as_none() {
        let path = std::env::temp_dir().join("dlssg-ipc-missing/status.json");
        assert!(read_status(&path).is_none());
    }

    #[test]
    fn publish_writes_the_file_even_without_a_pipe() {
        let directory =
            std::env::temp_dir().join(format!("dlssg-ipc-{}-publish", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let path = status_path(&directory);
        let status = StatusSnapshot::new("fixture", "recovering", "恢复中");
        let pushed = publish(None, &path, &status).expect("发布");
        assert!(!pushed, "没有管道时不应声称推送成功");
        assert_eq!(
            read_status(&path),
            Some(status),
            "管道不可用也要能读到同一阶段"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn pipe_name_is_game_scoped() {
        assert_eq!(
            pipe_name("steam-4356430"),
            "\\\\.\\pipe\\DlssgLauncher-steam-4356430"
        );
    }
}
