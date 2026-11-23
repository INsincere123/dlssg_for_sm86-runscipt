//! 已安装游戏记录（M3-03 的注册表部分）。
//!
//! 只读取、不改动，也不需要管理员：Steam / GOG / Epic 各自把安装位置记在不同地方。
//!
//! - Steam：`HKCU\Software\Valve\Steam` 的 `SteamPath`，或 `HKLM\SOFTWARE\WOW6432Node\Valve\Steam`
//!   的 `InstallPath` → `<Steam>\steamapps\common`
//! - GOG：`HKLM\SOFTWARE\WOW6432Node\GOG.com\Games\<id>` 的 `path`（每个游戏一个子键）
//! - Epic：`%ProgramData%\Epic\UnrealEngineLauncher\LauncherInstalled.dat` 里的 `InstallLocation`
//!
//! 读不到的来源直接跳过（不报错），返回结果按路径去重。

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use launcher_core::discovery::{DiscoveryRoot, merge_roots};

type Handle = isize;
pub(crate) const HKEY_CURRENT_USER: Handle = 0x8000_0001u32 as Handle;
pub(crate) const HKEY_LOCAL_MACHINE: Handle = 0x8000_0002u32 as Handle;
pub(crate) const KEY_READ: u32 = 0x0002_0019;
const KEY_WOW64_64KEY: u32 = 0x0100;
const REG_SZ: u32 = 1;
const REG_EXPAND_SZ: u32 = 2;
const ERROR_SUCCESS: i32 = 0;
const ERROR_NO_MORE_ITEMS: i32 = 259;

#[link(name = "advapi32")]
unsafe extern "system" {
    fn RegOpenKeyExW(
        key: Handle,
        sub_key: *const u16,
        options: u32,
        desired: u32,
        result: *mut Handle,
    ) -> i32;
    fn RegQueryValueExW(
        key: Handle,
        value_name: *const u16,
        reserved: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        size: *mut u32,
    ) -> i32;
    fn RegEnumKeyExW(
        key: Handle,
        index: u32,
        name: *mut u16,
        name_size: *mut u32,
        reserved: *mut u32,
        class: *mut u16,
        class_size: *mut u32,
        last_write: *mut c_void,
    ) -> i32;
    fn RegCloseKey(key: Handle) -> i32;
    fn RegSetValueExW(
        key: Handle,
        name: *const u16,
        reserved: u32,
        kind: u32,
        data: *const u8,
        size: u32,
    ) -> i32;
}

fn wide(value: &str) -> Vec<u16> {
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

pub(crate) fn open(root: Handle, sub_key: &str) -> Option<Handle> {
    let name = wide(sub_key);
    for access in [KEY_READ | KEY_WOW64_64KEY, KEY_READ] {
        let mut handle: Handle = 0;
        let status = unsafe { RegOpenKeyExW(root, name.as_ptr(), 0, access, &mut handle) };
        if status == ERROR_SUCCESS && handle != 0 {
            return Some(handle);
        }
    }
    None
}

pub(crate) fn read_string(root: Handle, sub_key: &str, value_name: &str) -> Option<String> {
    let handle = open(root, sub_key)?;
    let name = wide(value_name);
    let mut kind: u32 = 0;
    let mut size: u32 = 0;
    let status = unsafe {
        RegQueryValueExW(
            handle,
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut kind,
            std::ptr::null_mut(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS || size == 0 || (kind != REG_SZ && kind != REG_EXPAND_SZ) {
        unsafe { RegCloseKey(handle) };
        return None;
    }
    let mut buffer = vec![0u8; size as usize];
    let status = unsafe {
        RegQueryValueExW(
            handle,
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut kind,
            buffer.as_mut_ptr(),
            &mut size,
        )
    };
    unsafe { RegCloseKey(handle) };
    if status != ERROR_SUCCESS {
        return None;
    }
    let units: Vec<u16> = buffer
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    let text = String::from_utf16_lossy(&units);
    (!text.trim().is_empty()).then(|| text.trim().to_owned())
}

pub(crate) fn sub_keys(root: Handle, sub_key: &str) -> Vec<String> {
    let Some(handle) = open(root, sub_key) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    let mut index = 0u32;
    loop {
        let mut buffer = [0u16; 256];
        let mut size = buffer.len() as u32;
        let status = unsafe {
            RegEnumKeyExW(
                handle,
                index,
                buffer.as_mut_ptr(),
                &mut size,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if status == ERROR_NO_MORE_ITEMS {
            break;
        }
        if status != ERROR_SUCCESS {
            break;
        }
        names.push(String::from_utf16_lossy(&buffer[..size as usize]));
        index += 1;
    }
    unsafe { RegCloseKey(handle) };
    names
}

/// Steam 的游戏根目录：`<Steam>\steamapps\common`。
pub fn steam_common_root() -> Option<PathBuf> {
    let steam = read_string(HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamPath")
        .or_else(|| {
            read_string(
                HKEY_LOCAL_MACHINE,
                r"SOFTWARE\WOW6432Node\Valve\Steam",
                "InstallPath",
            )
        })
        .or_else(|| read_string(HKEY_LOCAL_MACHINE, r"SOFTWARE\Valve\Steam", "InstallPath"))?;
    Some(PathBuf::from(steam).join("steamapps").join("common"))
}

/// GOG 已安装游戏的目录（每个子键一个 `path`）。
pub fn gog_install_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    for base in [
        r"SOFTWARE\WOW6432Node\GOG.com\Games",
        r"SOFTWARE\GOG.com\Games",
    ] {
        for key in sub_keys(HKEY_LOCAL_MACHINE, base) {
            if let Some(path) = read_string(HKEY_LOCAL_MACHINE, &format!("{base}\\{key}"), "path") {
                directories.push(PathBuf::from(path));
            }
        }
    }
    directories
}

/// 从 Epic 的 `LauncherInstalled.dat` 文本里抽出 `InstallLocation`。
pub fn epic_install_directories(text: &str) -> Vec<PathBuf> {
    const KEY: &str = "\"InstallLocation\"";
    let mut directories = Vec::new();
    let mut rest = text;
    while let Some(index) = rest.find(KEY) {
        rest = &rest[index + KEY.len()..];
        let Some(colon) = rest.find(':') else { break };
        rest = &rest[colon + 1..];
        let Some(start) = rest.find('"') else { break };
        let after = &rest[start + 1..];
        let Some(end) = after.find('"') else { break };
        let value = &after[..end];
        if !value.trim().is_empty() {
            directories.push(PathBuf::from(value.replace("\\\\", "\\")));
        }
        rest = &after[end + 1..];
    }
    directories
}

/// Epic 的安装记录文件位置。
pub fn epic_manifest_path() -> Option<PathBuf> {
    let program_data = std::env::var_os("ProgramData")?;
    Some(
        Path::new(&program_data)
            .join("Epic")
            .join("UnrealEngineLauncher")
            .join("LauncherInstalled.dat"),
    )
}

/// Steam 安装目录（`Steam.exe` 所在目录）。
pub fn steam_install_directory() -> Option<PathBuf> {
    let steam = read_string(HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamPath")
        .or_else(|| {
            read_string(
                HKEY_LOCAL_MACHINE,
                r"SOFTWARE\WOW6432Node\Valve\Steam",
                "InstallPath",
            )
        })
        .or_else(|| read_string(HKEY_LOCAL_MACHINE, r"SOFTWARE\Valve\Steam", "InstallPath"))?;
    Some(PathBuf::from(steam))
}

/// 全部 Steam 库的 `steamapps` 目录（主库 + `libraryfolders.vdf` 登记的库）。
pub fn steam_library_steamapps() -> Vec<PathBuf> {
    let Some(steam) = steam_install_directory() else {
        return Vec::new();
    };
    let executable = steam.join("Steam.exe");
    match launcher_core::steam::library_directories(&executable) {
        Ok(directories) => directories
            .into_iter()
            .filter(|path| path.is_dir())
            .collect(),
        Err(_) => {
            let fallback = steam.join("steamapps");
            if fallback.is_dir() {
                vec![fallback]
            } else {
                Vec::new()
            }
        }
    }
}

/// GOG 的 `(显示名, 安装目录)`：每个子键的 `gameName` + `path`。
pub fn gog_games() -> Vec<(String, PathBuf)> {
    let mut games: Vec<(String, PathBuf)> = Vec::new();
    for base in [
        r"SOFTWARE\WOW6432Node\GOG.com\Games",
        r"SOFTWARE\GOG.com\Games",
    ] {
        for key in sub_keys(HKEY_LOCAL_MACHINE, base) {
            let path = format!("{base}\\{key}");
            let Some(directory) = read_string(HKEY_LOCAL_MACHINE, &path, "path") else {
                continue;
            };
            let name =
                read_string(HKEY_LOCAL_MACHINE, &path, "gameName").unwrap_or_else(|| key.clone());
            games.push((name, PathBuf::from(directory)));
        }
    }
    games
}

/// Epic：`(AppName, 显示名, 安装目录)`。显示名从 `Manifests\<AppName>.item` 里读，
/// 读不到就是 `None`（调用方退回 AppName）。
pub fn epic_games() -> Vec<(String, Option<String>, PathBuf)> {
    let Some(manifest) = epic_manifest_path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(&manifest) else {
        return Vec::new();
    };
    let mut games: Vec<(String, Option<String>, PathBuf)> = Vec::new();
    for (app_name, install) in epic_pairs(&text) {
        let display = epic_display_name(&app_name);
        games.push((app_name, display, PathBuf::from(install)));
    }
    games
}

/// 从 `LauncherInstalled.dat` 里抽 `(AppName, InstallLocation)` 配对。
///
/// 两个键在同一个对象里，顺序固定（`AppName` 先于 `InstallLocation` 或反之都可能），
/// 因此按对象顺序扫描：遇到 `AppName` 记住，遇到 `InstallLocation` 配对。
pub fn epic_pairs(text: &str) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut pending: Option<String> = None;
    let mut index = 0usize;
    while index < text.len() {
        let rest = &text[index..];
        let Some(quote) = rest.find('"') else {
            break;
        };
        let after = &rest[quote + 1..];
        let Some(end) = after.find('"') else {
            break;
        };
        let key = &after[..end];
        index += quote + 1 + end + 1;
        if key != "AppName" && key != "InstallLocation" {
            continue;
        }
        let rest = &text[index..];
        let Some(colon) = rest.find(':') else {
            break;
        };
        let rest = &rest[colon + 1..];
        let Some(start) = rest.find('"') else {
            break;
        };
        let after = &rest[start + 1..];
        let Some(end) = after.find('"') else {
            break;
        };
        let value = after[..end].replace("\\\\", "\\");
        index += colon + 1 + start + 1 + end + 1;
        if key == "AppName" {
            pending = Some(value);
        } else if let Some(app) = pending.take() {
            pairs.push((app, value));
        }
    }
    pairs
}

/// Epic 清单目录里的 `.item` 文件，读 `DisplayName`。
fn epic_display_name(app_name: &str) -> Option<String> {
    let program_data = std::env::var_os("ProgramData")?;
    let path = Path::new(&program_data)
        .join("Epic")
        .join("EpicGamesLauncher")
        .join("Data")
        .join("Manifests")
        .join(format!("{app_name}.item"));
    let text = std::fs::read_to_string(path).ok()?;
    let needle = "\"DisplayName\"";
    let start = text.find(needle)? + needle.len();
    let rest = &text[start..];
    let colon = rest.find(':')?;
    let rest = &rest[colon + 1..];
    let quote = rest.find('"')?;
    let after = &rest[quote + 1..];
    let end = after.find('"')?;
    let value = after[..end].replace("\\\\", "\\");
    (!value.trim().is_empty()).then_some(value)
}

/// 注册表卸载项（只取定位内置游戏要用的字段）。
pub fn uninstall_entries() -> Vec<launcher_core::builtin::UninstallEntry> {
    use launcher_core::builtin::UninstallEntry;

    const BASES: [(&str, Handle); 3] = [
        (
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
            HKEY_LOCAL_MACHINE,
        ),
        (
            r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
            HKEY_LOCAL_MACHINE,
        ),
        (
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
            HKEY_CURRENT_USER,
        ),
    ];
    let mut entries: Vec<UninstallEntry> = Vec::new();
    for (base, root) in BASES {
        for key in sub_keys(root, base) {
            let path = format!("{base}\\{key}");
            let Some(display_name) = read_string(root, &path, "DisplayName") else {
                continue;
            };
            entries.push(UninstallEntry {
                display_name,
                install_location: read_string(root, &path, "InstallLocation"),
                install_path: read_string(root, &path, "InstallPath"),
                display_icon: read_string(root, &path, "DisplayIcon"),
            });
        }
    }
    entries
}

/// 汇总所有来源（去重后按 Steam → Epic → GOG 顺序）。
pub fn installed_game_roots() -> Vec<DiscoveryRoot> {
    let mut roots: Vec<DiscoveryRoot> = Vec::new();
    if let Some(path) = steam_common_root() {
        if path.is_dir() {
            roots.push(DiscoveryRoot {
                label: "Steam".to_owned(),
                path,
            });
        }
    }
    if let Some(manifest) = epic_manifest_path() {
        if let Ok(text) = std::fs::read_to_string(&manifest) {
            for path in epic_install_directories(&text) {
                if path.is_dir() {
                    roots.push(DiscoveryRoot {
                        label: "Epic".to_owned(),
                        path,
                    });
                }
            }
        }
    }
    for path in gog_install_directories() {
        if path.is_dir() {
            roots.push(DiscoveryRoot {
                label: "GOG".to_owned(),
                path,
            });
        }
    }
    merge_roots(roots)
}

/// 写入一个 REG_SZ 值（需要管理员权限；调用方必须先备份）。
pub(crate) fn write_string(
    root: Handle,
    sub_key: &str,
    value_name: &str,
    value: &str,
) -> launcher_core::Result<()> {
    let handle = open(root, sub_key)
        .ok_or_else(|| launcher_core::Error::invalid(format!("注册表键不存在：{sub_key}")))?;
    let name = wide(value_name);
    let mut data: Vec<u8> = value
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    data.extend_from_slice(&[0, 0]);
    let status = unsafe {
        RegSetValueExW(
            handle,
            name.as_ptr(),
            0,
            REG_SZ,
            data.as_ptr(),
            data.len() as u32,
        )
    };
    unsafe { RegCloseKey(handle) };
    if status != ERROR_SUCCESS {
        return Err(launcher_core::Error::invalid(format!(
            "写入注册表失败（{sub_key}\\{value_name}）：错误码 {status}"
        )));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epic_text_extraction_handles_escaped_paths() {
        let text = r#"{"InstallationList":[
            {"InstallLocation":"D:\\\\Games\\\\Epic\\\\GameA","AppName":"A"},
            {"InstallLocation":"E:/Games/GameB"}]}"#;
        let directories = epic_install_directories(text);
        assert_eq!(
            directories,
            vec![
                PathBuf::from(r"D:\Games\Epic\GameA"),
                PathBuf::from("E:/Games/GameB"),
            ]
        );
        assert!(epic_install_directories("{\"Other\":1}").is_empty());
    }

    #[test]
    fn reading_the_registry_does_not_panic() {
        // 本机可能没装其中任何一个；断言只要求"能读、不崩、路径存在"。
        let roots = installed_game_roots();
        for root in &roots {
            assert!(
                ["Steam", "Epic", "GOG"].contains(&root.label.as_str()),
                "来源标签异常：{}",
                root.label
            );
            assert!(
                root.path.is_dir(),
                "返回的根目录必须存在：{}",
                root.path.display()
            );
        }
        // Steam 根目录的形态必须正确（存在时）
        if let Some(path) = steam_common_root() {
            assert!(path.ends_with("steamapps\\common") || path.ends_with("steamapps/common"));
        }
    }
}
