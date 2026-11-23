//! Steam 库与 AppID 识别（M3-01）。
//!
//! 逐条对齐 `启动脚本/scripts/common/SteamLauncher.ps1` 的
//! `Get-SteamLibraryDirectories` / `Find-SteamGameDirectory` / `Resolve-SteamAppId`：
//!
//! - 库目录 = `<Steam>\steamapps` ∪ `libraryfolders.vdf` 里所有**绝对**的 `"path"` 的 `steamapps`
//! - 清单只认 `appmanifest_<AppID>.acf`，且文件里 `"installdir"` 必须**恰好一行**、**不得是绝对路径**
//! - 安装目录必须落在 `<库>\steamapps\common\` 之内（大小写不敏感）
//! - 游戏自己的 `steamapps` 也是权威来源，即使它没登记在 `libraryfolders.vdf`
//! - 识别 AppID 时清单里的 `"appid"` 必须恰好一行，且与**清单文件名**一致，否则报"清单无效"
//! - 匹配数不为 1 时报"无法唯一识别"，并把候选逐个列出，**不猜**
//!
//! VDF/ACF 只按行解析 `^\s*"键"\s+"值"\s*$`，与 PS 的正则等价；不做完整 VDF 语法树。

use std::path::{Component, Path, PathBuf};

use crate::error::{Error, Result};
use crate::path::PathContext;

/// `steamapps` 的固定名字。
pub const STEAMAPPS: &str = "steamapps";
/// 库内游戏目录的固定名字。
pub const COMMON: &str = "common";

/// 一条清单的识别结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppIdMatch {
    pub app_id: String,
    pub manifest: PathBuf,
}

/// 启动参数：`steam.exe -applaunch <AppID>`。
pub fn launch_arguments(app_id: &str) -> Result<Vec<String>> {
    if !is_valid_app_id(app_id) {
        return Err(Error::invalid(format!(
            "AppID 必须是正整数（只允许 1-9 开头）：{app_id}"
        )));
    }
    Ok(vec!["-applaunch".to_owned(), app_id.to_owned()])
}

fn is_valid_app_id(value: &str) -> bool {
    let mut characters = value.chars();
    match characters.next() {
        Some(first) if first.is_ascii_digit() && first != '0' => {}
        _ => return false,
    }
    characters.all(|character| character.is_ascii_digit())
}

/// `<Steam>\steamapps`：`Steam.exe` 所在目录的 `steamapps`。
pub fn primary_library(steam_executable: &Path) -> Result<PathBuf> {
    let parent = steam_executable.parent().ok_or_else(|| {
        Error::invalid(format!(
            "Steam 可执行文件路径没有父目录：{}",
            steam_executable.display()
        ))
    })?;
    Ok(parent.join(STEAMAPPS))
}

/// 全部库的 `steamapps` 目录：主库 + `libraryfolders.vdf` 登记的库，去重排序。
pub fn library_directories(steam_executable: &Path) -> Result<Vec<PathBuf>> {
    let context = PathContext::current()?;
    let main = primary_library(steam_executable)?;
    let mut directories: Vec<String> = vec![clean(&context, &main.to_string_lossy())?];
    let folders = main.join("libraryfolders.vdf");
    if folders.is_file() {
        let text = std::fs::read_to_string(&folders).map_err(|error| Error::io(&folders, error))?;
        for value in vdf_values(&text, "path") {
            let library = unescape(&value);
            if !is_rooted(&library) {
                continue;
            }
            let joined = Path::new(&library).join(STEAMAPPS);
            directories.push(clean(&context, &joined.to_string_lossy())?);
        }
    }
    directories.sort_by_key(|path| path.to_lowercase());
    directories.dedup_by(|left, right| left.to_lowercase() == right.to_lowercase());
    Ok(directories.into_iter().map(PathBuf::from).collect())
}

/// 按 AppID 找安装目录。返回 0、1 或多个（多个表示库之间重复，调用方必须报错而不是挑一个）。
pub fn find_game_directory(steam_executable: &Path, app_id: &str) -> Result<Vec<PathBuf>> {
    if !is_valid_app_id(app_id) {
        return Err(Error::invalid(format!(
            "AppID 必须是正整数（只允许 1-9 开头）：{app_id}"
        )));
    }
    let context = PathContext::current()?;
    let mut found: Vec<PathBuf> = Vec::new();
    for library in library_directories(steam_executable)? {
        let manifest = library.join(format!("appmanifest_{app_id}.acf"));
        if !manifest.is_file() {
            continue;
        }
        let text =
            std::fs::read_to_string(&manifest).map_err(|error| Error::io(&manifest, error))?;
        let install_directory = match single_value(&text, "installdir") {
            Some(value) => unescape(&value),
            None => continue,
        };
        if is_rooted(&install_directory) {
            continue;
        }
        let common = library.join(COMMON);
        let game = match clean(&context, &common.join(&install_directory).to_string_lossy()) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if !inside(&context, &game, &common)? {
            continue;
        }
        if !Path::new(&game).is_dir() {
            continue;
        }
        match resolve_app_id(&context, Path::new(&game), steam_executable) {
            Ok(resolved) if resolved.app_id == app_id => found.push(PathBuf::from(game)),
            // 单个库的清单无效不应中断整体搜索：与 PS 一样继续看下一个库。
            Ok(_) => {}
            Err(error) if error.to_string().contains("清单无效") => {}
            Err(error) => return Err(error),
        }
    }
    found.sort_by_key(|path| path.to_string_lossy().to_lowercase());
    found.dedup_by(|left, right| {
        left.to_string_lossy().to_lowercase() == right.to_string_lossy().to_lowercase()
    });
    Ok(found)
}

/// 由游戏目录反查 AppID。游戏自己的库也参与匹配（即使未登记在 `libraryfolders.vdf`）。
pub fn resolve_app_id(
    context: &PathContext,
    game_directory: &Path,
    steam_executable: &Path,
) -> Result<AppIdMatch> {
    resolve_app_id_with(context, game_directory, steam_executable)
}

fn resolve_app_id_with(
    context: &PathContext,
    game_directory: &Path,
    steam_executable: &Path,
) -> Result<AppIdMatch> {
    let game = clean(context, &game_directory.to_string_lossy())?;
    let game_path = Path::new(&game);

    let mut libraries: Vec<PathBuf> = Vec::new();
    // 游戏自身的库优先，且不要求已登记。
    if let (Some(common), Some(apps)) = (
        game_path.parent(),
        game_path.parent().and_then(Path::parent),
    ) {
        if leaf_eq(common, COMMON) && leaf_eq(apps, STEAMAPPS) {
            libraries.push(apps.to_path_buf());
        }
    }
    for library in library_directories(steam_executable)? {
        libraries.push(library);
    }

    let mut matches: Vec<AppIdMatch> = Vec::new();
    for library in libraries {
        if !library.is_dir() {
            continue;
        }
        let mut manifests: Vec<PathBuf> = std::fs::read_dir(&library)
            .map_err(|error| Error::io(&library, error))?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file()
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .map(|name| {
                            let lower = name.to_lowercase();
                            lower.starts_with("appmanifest_") && lower.ends_with(".acf")
                        })
                        .unwrap_or(false)
            })
            .collect();
        manifests.sort();

        for manifest in manifests {
            let text =
                std::fs::read_to_string(&manifest).map_err(|error| Error::io(&manifest, error))?;
            let install_directory = match single_value(&text, "installdir") {
                Some(value) => unescape(&value),
                None => continue,
            };
            if is_rooted(&install_directory) {
                continue;
            }
            let common = library.join(COMMON);
            let candidate = match clean(context, &common.join(&install_directory).to_string_lossy())
            {
                Ok(value) => value,
                Err(_) => continue,
            };
            if !inside(context, &candidate, &common)?
                || candidate.to_lowercase() != game.to_lowercase()
            {
                continue;
            }
            let app_id = match single_value(&text, "appid") {
                Some(value) => value,
                None => {
                    return Err(Error::invalid(format!(
                        "匹配游戏的 Steam 清单无效，请检查：{}",
                        manifest.display()
                    )));
                }
            };
            let expected = format!("appmanifest_{app_id}.acf");
            let actual = manifest
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            if !actual.eq_ignore_ascii_case(&expected) {
                return Err(Error::invalid(format!(
                    "匹配游戏的 Steam 清单无效，请检查：{}",
                    manifest.display()
                )));
            }
            matches.push(AppIdMatch { app_id, manifest });
        }
    }

    matches.sort_by(|left, right| left.manifest.cmp(&right.manifest));
    matches.dedup_by(|left, right| {
        left.manifest.to_string_lossy().to_lowercase()
            == right.manifest.to_string_lossy().to_lowercase()
    });

    match matches.len() {
        1 => Ok(matches.remove(0)),
        count => {
            let details = matches
                .iter()
                .map(|item| format!("{}: {}", item.app_id, item.manifest.display()))
                .collect::<Vec<_>>()
                .join("; ");
            Err(Error::invalid(format!(
                "无法唯一识别游戏 AppID（匹配 {count} 项）：{game}。请通过 Steam 安装/识别该目录，或显式填写 -AppId。{details}"
            )))
        }
    }
}

/// 逐行取 `"键" "值"` 里的值（含转义，未反转义）。
fn vdf_values(text: &str, key: &str) -> Vec<String> {
    let mut values = Vec::new();
    for line in text.lines() {
        if let Some(value) = parse_pair(line, key) {
            values.push(value);
        }
    }
    values
}

/// 恰好出现一次才返回；出现零次或多次都算不唯一（与 PS `$dirs.Count -ne 1` 一致）。
fn single_value(text: &str, key: &str) -> Option<String> {
    let values = vdf_values(text, key);
    if values.len() == 1 {
        values.into_iter().next()
    } else {
        None
    }
}

/// 解析一行 `   "key"  "value"  `；值里允许 `\"` 与 `\\`。
fn parse_pair(line: &str, key: &str) -> Option<String> {
    let trimmed = line.trim();
    if !trimmed.starts_with('"') {
        return None;
    }
    let (parsed_key, rest) = quoted(trimmed)?;
    if parsed_key != key {
        return None;
    }
    let rest = rest.trim_start();
    if !rest.starts_with('"') {
        return None;
    }
    let (value, tail) = quoted(rest)?;
    if !tail.trim().is_empty() {
        return None;
    }
    Some(value)
}

/// 读一个带转义的引号字符串，返回内容与剩余部分。
fn quoted(text: &str) -> Option<(String, &str)> {
    let mut characters = text.char_indices();
    match characters.next() {
        Some((_, '"')) => {}
        _ => return None,
    }
    let mut value = String::new();
    while let Some((index, current)) = characters.next() {
        match current {
            '\\' => {
                let (_, escaped) = characters.next()?;
                value.push('\\');
                value.push(escaped);
            }
            '"' => return Some((value, &text[index + 1..])),
            other => value.push(other),
        }
    }
    None
}

fn unescape(value: &str) -> String {
    value.replace("\\\\", "\\").replace("\\\"", "\"")
}

/// PS 的 `[IO.Path]::IsPathRooted`：有盘符前缀或根目录即算绝对（`C:foo` 也算）。
fn is_rooted(value: &str) -> bool {
    let path = Path::new(value);
    matches!(
        path.components().next(),
        Some(Component::Prefix(_)) | Some(Component::RootDir)
    )
}

/// `GetFullPath(x).TrimEnd('\')`，保留大小写与根目录形式。
fn clean(context: &PathContext, value: &str) -> Result<String> {
    let full = context.full_path(value)?;
    let trimmed = full.trim_end_matches('\\');
    if trimmed.is_empty() || trimmed.len() == 2 && trimmed.ends_with(':') {
        return Ok(format!("{trimmed}\\"));
    }
    Ok(trimmed.to_owned())
}

/// 候选是否位于某目录之内（大小写不敏感，且要求边界是 `\`）。
fn inside(context: &PathContext, candidate: &str, directory: &Path) -> Result<bool> {
    let root = clean(context, &directory.to_string_lossy())?;
    let prefix = format!("{}\\", root.to_lowercase());
    Ok(candidate.to_lowercase().starts_with(&prefix))
}

fn leaf_eq(path: &Path, expected: &str) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case(expected))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VDF: &str = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
	}
	"2"
	{
		"path"		"relative\\Library"
	}
	"contentstatsid"		"-1234567890"
}
"#;

    #[test]
    fn vdf_paths_are_unescaped_and_only_rooted_ones_kept() {
        let values: Vec<String> = vdf_values(VDF, "path")
            .iter()
            .map(|value| unescape(value))
            .collect();
        assert_eq!(
            values,
            vec![
                r"C:\Program Files (x86)\Steam".to_owned(),
                r"D:\SteamLibrary".to_owned(),
                r"relative\Library".to_owned(),
            ]
        );
        assert!(!is_rooted(&values[2]));
        assert!(is_rooted(&values[0]));
    }

    #[test]
    fn pair_parser_requires_exactly_one_pair_per_line() {
        assert_eq!(
            parse_pair("\t\t\"path\"\t\t\"D:\\\\SteamLibrary\"", "path"),
            Some(r"D:\\SteamLibrary".to_owned())
        );
        assert_eq!(parse_pair("\"path\" \"a\" \"b\"", "path"), None);
        assert_eq!(parse_pair("// \"path\" \"a\"", "path"), None);
        assert_eq!(parse_pair("\"other\" \"a\"", "path"), None);
    }

    #[test]
    fn app_id_must_be_a_positive_integer() {
        assert!(launch_arguments("4356430").is_ok());
        assert_eq!(
            launch_arguments("4356430").expect("参数"),
            vec!["-applaunch".to_owned(), "4356430".to_owned()]
        );
        for bad in ["", "0", "01", "abc", "12a", "-1"] {
            assert!(launch_arguments(bad).is_err(), "应拒绝 {bad}");
        }
    }

    #[test]
    fn single_value_rejects_duplicates() {
        let text = "\"installdir\" \"a\"\n\"installdir\" \"b\"\n";
        assert_eq!(single_value(text, "installdir"), None);
        assert_eq!(
            single_value("\"installdir\" \"a\"\n", "installdir"),
            Some("a".to_owned())
        );
    }
}
