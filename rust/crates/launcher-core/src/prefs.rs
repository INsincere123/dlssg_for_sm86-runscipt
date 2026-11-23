//! 本机偏好目录（PARITY C11）。
//!
//! 对应现有实现 `Get-PortableDataDirectory`（`PortableLauncher.ps1:5-11`）：
//!
//! ```text
//! key = UPPERHEX(SHA256(UTF8(GetFullPath(root).ToLowerInvariant())))[0..16]
//! dir = %LOCALAPPDATA%\<数据目录名>\<key>
//! ```
//!
//! 数据目录名在新程序里改成 `DlssgLauncher`（PARITY D-3），旧名 `DlssgSteamLauncher`
//! 只读兼容一次；`runs/<guid>` 结构保持不变。

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// 新程序的数据目录名。
pub const DATA_DIRECTORY_NAME: &str = "DlssgLauncher";

/// 旧 WinForms 入口的数据目录名（只读兼容）。
pub const LEGACY_DATA_DIRECTORY_NAME: &str = "DlssgSteamLauncher";

/// 启动器根目录 → 16 位十六进制键。
pub fn path_key(root: &Path) -> Result<String> {
    let text = root
        .to_str()
        .ok_or_else(|| Error::invalid(format!("路径不是合法 UTF-8：{}", root.display())))?;
    let full = crate::path::PathContext::current()?.full_path(text)?;
    let hash = crate::path::sha256_upper(full.to_lowercase().as_bytes());
    Ok(hash[..16].to_owned())
}

/// 在给定 `%LOCALAPPDATA%` 基目录下拼出数据目录（便于测试注入）。
pub fn data_directory_in(base: &Path, directory_name: &str, root: &Path) -> Result<PathBuf> {
    Ok(base.join(directory_name).join(path_key(root)?))
}

/// 新程序的数据目录。
pub fn data_directory(root: &Path) -> Result<PathBuf> {
    let base = local_app_data()?;
    data_directory_in(&base, DATA_DIRECTORY_NAME, root)
}

/// 旧入口的数据目录（只读兼容）。
pub fn legacy_data_directory(root: &Path) -> Result<PathBuf> {
    let base = local_app_data()?;
    data_directory_in(&base, LEGACY_DATA_DIRECTORY_NAME, root)
}

/// `preferences.json` 的位置。
pub fn preferences_path(root: &Path) -> Result<PathBuf> {
    Ok(data_directory(root)?.join("preferences.json"))
}

/// 每游戏设置目录：`games/<gameId>.json`。
pub fn game_settings_path(root: &Path, game_id: &str) -> Result<PathBuf> {
    Ok(data_directory(root)?
        .join("games")
        .join(format!("{game_id}.json")))
}

/// 运行请求目录：`runs/<guid>`。
pub fn run_directory(root: &Path, guid: &str) -> Result<PathBuf> {
    if guid.is_empty()
        || !guid
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(Error::invalid(format!("运行请求标识非法：{guid}")));
    }
    Ok(data_directory(root)?.join("runs").join(guid))
}

fn local_app_data() -> Result<PathBuf> {
    match std::env::var_os("LOCALAPPDATA") {
        Some(value) if !value.is_empty() => Ok(PathBuf::from(value)),
        _ => Err(Error::unavailable(
            "环境变量 LOCALAPPDATA 未设置，无法定位本机偏好目录。",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_matches_the_powershell_formula() {
        // 由 PowerShell 实测得到：D:\Games\dlssg_for_sm86 → 0832029A5839CB2D
        let key = path_key(Path::new(r"D:\Games\dlssg_for_sm86")).expect("键");
        assert_eq!(key, "0832029A5839CB2D");
        assert_eq!(key.len(), 16);
    }

    #[test]
    fn key_is_case_insensitive_and_ignores_trailing_separator() {
        let lower = path_key(Path::new(r"d:\games\dlssg_for_sm86")).expect("键");
        let with_separator = path_key(Path::new(r"D:\Games\dlssg_for_sm86\")).expect("键");
        assert_eq!(lower, "0832029A5839CB2D");
        assert_eq!(with_separator, "0832029A5839CB2D");
    }

    #[test]
    fn directory_layout_uses_the_injected_base() {
        let base = Path::new(r"C:\Users\someone\AppData\Local");
        let root = Path::new(r"D:\Games\dlssg_for_sm86");
        let directory = data_directory_in(base, DATA_DIRECTORY_NAME, root).expect("数据目录");
        assert_eq!(
            directory,
            PathBuf::from(r"C:\Users\someone\AppData\Local\DlssgLauncher\0832029A5839CB2D")
        );
        let legacy = data_directory_in(base, LEGACY_DATA_DIRECTORY_NAME, root).expect("旧数据目录");
        assert_eq!(
            legacy,
            PathBuf::from(r"C:\Users\someone\AppData\Local\DlssgSteamLauncher\0832029A5839CB2D")
        );
    }

    #[test]
    fn run_directory_rejects_unsafe_identifiers() {
        let root = Path::new(r"D:\Games\dlssg_for_sm86");
        if std::env::var_os("LOCALAPPDATA").is_none() {
            return; // 没有环境变量时跳过，不伪造结果
        }
        assert!(run_directory(root, "").is_err());
        assert!(run_directory(root, r"..\evil").is_err());
        let ok = run_directory(root, "0123456789abcdef0123456789abcdef").expect("合法标识");
        assert!(ok.ends_with("0123456789abcdef0123456789abcdef"));
        assert!(ok.to_string_lossy().replace('\\', "/").contains("/runs/"));
    }
}
