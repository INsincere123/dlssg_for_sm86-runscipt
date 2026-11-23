//! 直接 EXE 启动与统一启动方式（M3-02）。
//!
//! 对齐 `启动脚本/scripts/common/DirectLauncher.ps1`：
//!
//! - `Initialize-PortableLaunchMode`：缺 `LaunchMode` 按 Steam 兼容，缺 `LaunchExecutable` 取渲染 EXE；
//!   Direct 的本机标识必须是 `direct-<32 位小写十六进制>`
//! - `Get-PortableGameId`：Direct 用标识本身，Steam 用 `steam-<AppID>`（AppID 必须 `^[1-9][0-9]*$`）
//! - `Resolve-DirectExecutable`：必须是**游戏目录内的相对 EXE**，绝对路径与非 `.exe` 一律拒绝；
//!   不做存在性检查（与 PS 一致，缺失由上层报错）
//!
//! 启动请求以 `DirectLauncher.Tests.ps1:61` 的断言为准：工作目录是**游戏目录**，
//! Steam 走 `-applaunch <AppID>` 且隐藏窗口，Direct 直接起 EXE、正常窗口。

use std::path::{Component, Path, PathBuf};

use crate::error::{Error, Result};
use crate::path::PathContext;

/// 启动方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    Steam,
    Direct,
}

impl LaunchMode {
    /// 缺省即 Steam（旧偏好兼容）。
    pub fn from_optional(value: Option<&str>) -> Result<Self> {
        match value {
            None => Ok(Self::Steam),
            Some(text) if text.eq_ignore_ascii_case("Steam") => Ok(Self::Steam),
            Some(text) if text.eq_ignore_ascii_case("Direct") => Ok(Self::Direct),
            Some(_) => Err(Error::invalid("启动方式无效。")),
        }
    }

    pub fn as_text(self) -> &'static str {
        match self {
            Self::Steam => "Steam",
            Self::Direct => "Direct",
        }
    }
}

/// 一次启动请求（只描述，不执行；执行在 M3 之后的引擎流程里）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRequest {
    pub executable: PathBuf,
    /// 工作目录固定为游戏目录（见模块文档）。
    pub working_directory: PathBuf,
    pub arguments: Vec<String>,
    /// Steam 启动隐藏窗口，Direct 正常窗口。
    pub hidden: bool,
}

/// `Get-PortableGameId`：Direct 用本机标识，Steam 用 `steam-<AppID>`。
pub fn game_id(mode: LaunchMode, id: Option<&str>, app_id: Option<&str>) -> Result<String> {
    match mode {
        LaunchMode::Direct => {
            let id = id.ok_or_else(|| Error::invalid("直接启动游戏的本机标识无效。"))?;
            validate_direct_id(id)?;
            Ok(id.to_owned())
        }
        LaunchMode::Steam => {
            let app_id = app_id.ok_or_else(|| Error::invalid("Steam AppID无效。"))?;
            if !is_positive_integer(app_id) {
                return Err(Error::invalid("Steam AppID无效。"));
            }
            Ok(format!("steam-{app_id}"))
        }
    }
}

/// `Initialize-PortableLaunchMode` 里的标识校验。
pub fn validate_direct_id(id: &str) -> Result<()> {
    let valid = id.len() == 7 + 32
        && id.starts_with("direct-")
        && id[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if valid {
        Ok(())
    } else {
        Err(Error::invalid("直接启动游戏的本机标识无效。"))
    }
}

/// 缺 `LaunchExecutable` 时取渲染 EXE（返回实际应启动的可执行文件）。
pub fn launch_executable<'a>(declared: Option<&'a str>, render: &'a str) -> &'a str {
    match declared {
        Some(value) if !value.trim().is_empty() => value,
        _ => render,
    }
}

/// 生成 `direct-<32 位小写十六进制>` 标识。
pub fn new_direct_id(seed: &str) -> String {
    let hash = crate::path::sha256_upper(seed.as_bytes()).to_lowercase();
    format!("direct-{}", &hash[..32])
}

/// `Resolve-DirectExecutable`：把游戏目录内的相对 EXE 解析成绝对路径。
pub fn resolve_executable(
    context: &PathContext,
    game_directory: &Path,
    relative: &str,
    description: &str,
) -> Result<PathBuf> {
    if relative.trim().is_empty() || is_rooted(relative) {
        return Err(Error::invalid(format!(
            "{description} 必须是游戏目录内的相对EXE路径。"
        )));
    }
    let base = format!("{}\\", clean(context, game_directory)?);
    let candidate = context.full_path(&format!("{base}{relative}"))?;
    let lower_base = base.to_lowercase();
    let lower_candidate = candidate.to_lowercase();
    let extension_ok = Path::new(&candidate)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("exe"))
        .unwrap_or(false);
    if !lower_candidate.starts_with(&lower_base) || !extension_ok {
        return Err(Error::invalid(format!(
            "{description} 必须是游戏目录内的EXE。"
        )));
    }
    Ok(PathBuf::from(candidate))
}

/// 直接启动请求：`Start-Process -FilePath <exe> -WorkingDirectory <游戏目录> -WindowStyle Normal`。
pub fn direct_request(
    context: &PathContext,
    game_directory: &Path,
    relative_executable: &str,
) -> Result<LaunchRequest> {
    let executable = resolve_executable(context, game_directory, relative_executable, "程序")?;
    Ok(LaunchRequest {
        executable,
        working_directory: PathBuf::from(clean(context, game_directory)?),
        arguments: Vec::new(),
        hidden: false,
    })
}

/// Steam 启动请求：`-applaunch <AppID>`、隐藏窗口、工作目录为游戏目录。
pub fn steam_request(
    steam_executable: &Path,
    game_directory: &Path,
    app_id: &str,
) -> Result<LaunchRequest> {
    let context = PathContext::current()?;
    let arguments = crate::steam::launch_arguments(app_id)?;
    Ok(LaunchRequest {
        executable: PathBuf::from(context.full_path(&steam_executable.to_string_lossy())?),
        working_directory: PathBuf::from(clean(&context, game_directory)?),
        arguments,
        hidden: true,
    })
}

fn is_positive_integer(value: &str) -> bool {
    let mut characters = value.chars();
    match characters.next() {
        Some(first) if first.is_ascii_digit() && first != '0' => {}
        _ => return false,
    }
    characters.all(|character| character.is_ascii_digit())
}

fn is_rooted(value: &str) -> bool {
    matches!(
        Path::new(value).components().next(),
        Some(Component::Prefix(_)) | Some(Component::RootDir)
    )
}

fn clean(context: &PathContext, path: &Path) -> Result<String> {
    let full = context.full_path(&path.to_string_lossy())?;
    let trimmed = full.trim_end_matches('\\');
    if trimmed.is_empty() || trimmed.len() == 2 && trimmed.ends_with(':') {
        return Ok(format!("{trimmed}\\"));
    }
    Ok(trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> PathContext {
        PathContext::current().expect("上下文")
    }

    #[test]
    fn launch_mode_defaults_to_steam_and_rejects_unknown() {
        assert_eq!(
            LaunchMode::from_optional(None).expect("缺省"),
            LaunchMode::Steam
        );
        assert_eq!(
            LaunchMode::from_optional(Some("Direct")).expect("Direct"),
            LaunchMode::Direct
        );
        assert_eq!(
            LaunchMode::from_optional(Some("steam")).expect("大小写"),
            LaunchMode::Steam
        );
        assert!(LaunchMode::from_optional(Some("Epic")).is_err());
    }

    #[test]
    fn direct_id_must_be_lowercase_hex32() {
        assert!(validate_direct_id("direct-0123456789abcdef0123456789abcdef").is_ok());
        for bad in [
            "direct-0123456789ABCDEF0123456789abcdef",
            "direct-0123",
            "steam-4356430",
            "direct-0123456789abcdef0123456789abcdeg",
        ] {
            assert!(validate_direct_id(bad).is_err(), "应拒绝 {bad}");
        }
    }

    #[test]
    fn game_id_follows_the_mode() {
        assert_eq!(
            game_id(LaunchMode::Steam, None, Some("4356430")).expect("Steam"),
            "steam-4356430"
        );
        assert_eq!(
            game_id(
                LaunchMode::Direct,
                Some("direct-0123456789abcdef0123456789abcdef"),
                Some("99")
            )
            .expect("Direct"),
            "direct-0123456789abcdef0123456789abcdef"
        );
        assert!(game_id(LaunchMode::Steam, None, Some("0")).is_err());
        assert!(game_id(LaunchMode::Direct, None, None).is_err());
    }

    #[test]
    fn generated_direct_id_is_stable_and_valid() {
        let first = new_direct_id(r"D:\Games\SomeGame");
        assert_eq!(first, new_direct_id(r"D:\Games\SomeGame"), "同种子必须稳定");
        assert_ne!(
            first,
            new_direct_id(r"D:\Games\OtherGame"),
            "不同目录必须不同"
        );
        assert!(validate_direct_id(&first).is_ok(), "{first}");
    }

    #[test]
    fn relative_executable_is_resolved_inside_the_game_directory() {
        let context = context();
        let game = Path::new(r"D:\Games\SomeGame");
        let resolved =
            resolve_executable(&context, game, r"Binaries\Win64\Game.exe", "程序").expect("解析");
        assert_eq!(
            resolved,
            PathBuf::from(r"D:\Games\SomeGame\Binaries\Win64\Game.exe")
        );
    }

    #[test]
    fn escaping_or_absolute_or_non_exe_is_rejected() {
        let context = context();
        let game = Path::new(r"D:\Games\SomeGame");
        // 与 PS 一致：`..\x.exe` 先通过"相对"检查，再由前缀检查拒绝（第二条消息）；
        // 绝对路径与空串则在前一条就被拒。
        for (relative, expected) in [
            (r"..\outside.exe", "游戏目录内的EXE"),
            (r"C:\outside.exe", "相对EXE路径"),
            (r"bad.dll", "游戏目录内的EXE"),
            ("", "相对EXE路径"),
        ] {
            let error = resolve_executable(&context, game, relative, "程序").expect_err(relative);
            assert!(error.to_string().contains(expected), "{relative} → {error}");
        }
    }

    #[test]
    fn direct_request_uses_the_game_directory_and_a_normal_window() {
        let context = context();
        let game = Path::new(r"D:\Games\SomeGame");
        let request = direct_request(&context, game, r"Binaries\Win64\Game.exe").expect("请求");
        assert_eq!(
            request.working_directory,
            PathBuf::from(r"D:\Games\SomeGame")
        );
        assert!(request.arguments.is_empty());
        assert!(!request.hidden);
        assert!(request.executable.ends_with(r"Binaries\Win64\Game.exe"));
    }

    #[test]
    fn steam_request_hides_the_window_and_passes_the_app_id() {
        let request = steam_request(
            Path::new(r"C:\Program Files (x86)\Steam\steam.exe"),
            Path::new(r"D:\Games\BlackMyth"),
            "2358720",
        )
        .expect("请求");
        assert!(request.hidden);
        assert_eq!(
            request.arguments,
            vec!["-applaunch".to_owned(), "2358720".to_owned()]
        );
        assert_eq!(
            request.working_directory,
            PathBuf::from(r"D:\Games\BlackMyth")
        );
    }

    #[test]
    fn declared_launch_executable_falls_back_to_the_render_executable() {
        assert_eq!(launch_executable(None, "Render.exe"), "Render.exe");
        assert_eq!(launch_executable(Some("  "), "Render.exe"), "Render.exe");
        assert_eq!(
            launch_executable(Some("Launch.exe"), "Render.exe"),
            "Launch.exe"
        );
    }
}
