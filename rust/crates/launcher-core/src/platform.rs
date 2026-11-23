//! 平台清单扫描：从 Steam / GOG / Epic 的既有记录里列出**已安装的游戏**。
//!
//! 与 `discovery` 的分工：这里只解析"平台说装了哪些游戏"，不翻游戏目录；
//! 渲染 EXE 由调用方再用 `discovery::renderer_candidates` 确认。
//!
//! - Steam：`<steamapps>\appmanifest_<AppID>.acf` 里的 `appid` / `name` / `installdir`，
//!   安装目录必须落在 `<steamapps>\common\` 之内（与 `steam.rs` 的规则一致）
//! - GOG：注册表每个子键的 `gameName` + `path`
//! - Epic：`LauncherInstalled.dat` 的 `AppName` + `InstallLocation`，显示名优先用
//!   `Manifests\<AppName>.item` 里的 `DisplayName`
//!
//! 所有条目都会校验目录存在；重复目录按大小写不敏感去重，保留先出现的来源。

use std::path::{Path, PathBuf};

use crate::steam::COMMON;

/// 一台平台上发现的一个已安装游戏。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundGame {
    /// 平台上显示的名字（清单缺失时用目录名）。
    pub name: String,
    pub directory: PathBuf,
    /// 来源说明，例如 `Steam 库 D:\SteamLibrary`。
    pub source: String,
    /// Steam AppID（其它平台为 `None`）。
    pub app_id: Option<String>,
    /// Epic 的 `AppName`（`LauncherInstalled.dat` 里的值，其它平台为 `None`）。
    pub epic_app_name: Option<String>,
    /// `Steam` / `Epic` / `Direct`。
    pub launch_mode: String,
}

impl FoundGame {
    fn direct(name: String, directory: PathBuf, source: String) -> Self {
        Self {
            name,
            directory,
            source,
            app_id: None,
            epic_app_name: None,
            launch_mode: "Direct".to_owned(),
        }
    }
}

/// 目录去重用的键：统一分隔符 + 小写 + 去掉结尾反斜杠。
fn key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

/// 按目录去重（保留先出现的），顺手丢掉不存在的目录。
pub fn deduplicate(items: Vec<FoundGame>) -> Vec<FoundGame> {
    let mut result: Vec<FoundGame> = Vec::new();
    for item in items {
        if !item.directory.is_dir() {
            continue;
        }
        if result
            .iter()
            .any(|existing| key(&existing.directory) == key(&item.directory))
        {
            continue;
        }
        result.push(item);
    }
    result.sort_by_key(|item| item.name.to_lowercase());
    result
}

/// 扫一个 Steam 库的 `steamapps` 目录。
pub fn steam_games(steamapps: &Path) -> Vec<FoundGame> {
    let common = steamapps.join(COMMON);
    let library = steamapps
        .parent()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| steamapps.display().to_string());
    let source = format!("Steam 库 {library}");
    let entries = match std::fs::read_dir(steamapps) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };
    let mut games: Vec<FoundGame> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let lower = name.to_lowercase();
        if !path.is_file() || !lower.starts_with("appmanifest_") || !lower.ends_with(".acf") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(app_id) = single(&text, "appid") else {
            continue;
        };
        // 清单文件名与内部 appid 必须一致，否则这条记录不可信（与 steam.rs 一致）。
        if !name.eq_ignore_ascii_case(&format!("appmanifest_{app_id}.acf")) {
            continue;
        }
        let Some(install_directory) = single(&text, "installdir") else {
            continue;
        };
        if install_directory.is_empty() || is_rooted(&install_directory) {
            continue;
        }
        let directory = common.join(&install_directory);
        let display_name = single(&text, "name")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| install_directory.clone());
        if !directory.is_dir() || is_redistributable(&display_name, &install_directory) {
            continue;
        }
        games.push(FoundGame {
            name: display_name,
            directory,
            source: source.clone(),
            app_id: Some(app_id),
            epic_app_name: None,
            launch_mode: "Steam".to_owned(),
        });
    }
    games
}

/// Steam 里的"不是游戏"：运行库容器与 Proton/运行时。
///
/// 只为少让用户在一堆 `Steamworks Common Redistributables` 里挑，**不是**按目录名猜游戏。
pub fn is_redistributable(display_name: &str, install_directory: &str) -> bool {
    let name = display_name.to_lowercase();
    let directory = install_directory.to_lowercase();
    const NAME_FRAGMENTS: [&str; 6] = [
        "redistributable",
        "steam linux runtime",
        "proton",
        "steamworks sdk",
        "steamvr",
        "wallpaper engine - ",
    ];
    if NAME_FRAGMENTS
        .iter()
        .any(|fragment| name.contains(fragment) || directory.contains(fragment))
    {
        return true;
    }
    directory.starts_with("steamworks") || directory.starts_with("proton")
}

/// GOG：`(显示名, 安装目录)` 列表 → 条目。
pub fn gog_games(entries: &[(String, PathBuf)]) -> Vec<FoundGame> {
    entries
        .iter()
        .map(|(name, directory)| {
            FoundGame::direct(name.clone(), directory.clone(), "GOG 注册表".to_owned())
        })
        .collect()
}

/// Epic：`(AppName, 显示名可选, 安装目录)` → 条目。
///
/// Epic 游戏带着 `AppName` 走：启动要用 `com.epicgames.launcher://` 让 Epic 启动器拉起
/// （直接起 EXE 会绕开归属校验/在线服务，和 Steam 那边是同一类问题）。
pub fn epic_games(entries: &[(String, Option<String>, PathBuf)]) -> Vec<FoundGame> {
    entries
        .iter()
        .map(|(app_name, display, directory)| {
            let name = display
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| app_name.clone());
            FoundGame {
                name,
                directory: directory.clone(),
                source: "Epic 清单".to_owned(),
                app_id: None,
                epic_app_name: Some(app_name.clone()),
                launch_mode: "Epic".to_owned(),
            }
        })
        .collect()
}

/// Epic 启动 URI：`com.epicgames.launcher://apps/<AppName>?action=launch&silent=true`。
///
/// `AppName` 取自 `LauncherInstalled.dat`，不是显示名。只接受 `[A-Za-z0-9._-]`：
/// URI 直接交给系统（ShellExecute），不能让 AppName 里的分隔符拆坏路径或参数。
pub fn epic_launch_uri(app_name: &str) -> crate::error::Result<String> {
    let trimmed = app_name.trim();
    let valid = !trimmed.is_empty()
        && trimmed.len() <= 128
        && trimmed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if !valid {
        return Err(crate::error::Error::invalid(format!(
            "Epic AppName 无效：{app_name}"
        )));
    }
    Ok(format!(
        "com.epicgames.launcher://apps/{trimmed}?action=launch&silent=true"
    ))
}

/// 逐行取 `"键" "值"` 的值（恰好一行才算，与 `steam::single_value` 同规则）。
fn single(text: &str, key: &str) -> Option<String> {
    let mut values: Vec<String> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('"') {
            continue;
        }
        let mut parts = trimmed.split('"').skip(1).step_by(2);
        let Some(parsed_key) = parts.next() else {
            continue;
        };
        if parsed_key != key {
            continue;
        }
        if let Some(value) = parts.next() {
            values.push(value.replace("\\\\", "\\").replace("\\\"", "\""));
        }
    }
    if values.len() == 1 {
        values.into_iter().next()
    } else {
        None
    }
}

fn is_rooted(value: &str) -> bool {
    let path = Path::new(value);
    matches!(
        path.components().next(),
        Some(std::path::Component::Prefix(_)) | Some(std::path::Component::RootDir)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tree(PathBuf);

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tree(name: &str) -> Tree {
        let root =
            std::env::temp_dir().join(format!("dlssg-platform-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("临时目录");
        Tree(root)
    }

    fn acf(app_id: &str, name: &str, installdir: &str) -> String {
        format!(
            "\"AppState\"\n{{\n\t\"appid\"\t\t\"{app_id}\"\n\t\"name\"\t\t\"{name}\"\n\t\"installdir\"\t\t\"{installdir}\"\n}}\n"
        )
    }

    #[test]
    fn steam_manifests_become_games_with_app_id() {
        let tree = tree("steam");
        let steamapps = tree.0.join("steamapps");
        let common = steamapps.join("common");
        std::fs::create_dir_all(common.join("BlackMythWukong")).expect("游戏目录");
        std::fs::write(
            steamapps.join("appmanifest_2358720.acf"),
            acf("2358720", "黑神话：悟空", "BlackMythWukong"),
        )
        .expect("写清单");
        // 目录不存在 → 丢弃
        std::fs::write(
            steamapps.join("appmanifest_999.acf"),
            acf("999", "Missing Game", "Nope"),
        )
        .expect("写清单");
        // 文件名与 appid 不一致 → 丢弃
        std::fs::write(
            steamapps.join("appmanifest_111.acf"),
            acf("222", "Wrong Id", "BlackMythWukong"),
        )
        .expect("写清单");

        let games = steam_games(&steamapps);
        assert_eq!(games.len(), 1, "{games:?}");
        assert_eq!(games[0].name, "黑神话：悟空");
        assert_eq!(games[0].app_id.as_deref(), Some("2358720"));
        assert_eq!(games[0].launch_mode, "Steam");
        assert!(
            games[0].source.starts_with("Steam 库"),
            "{}",
            games[0].source
        );
        assert_eq!(games[0].directory, common.join("BlackMythWukong"));
    }

    #[test]
    fn absolute_installdir_is_rejected() {
        let tree = tree("absolute");
        let steamapps = tree.0.join("steamapps");
        std::fs::create_dir_all(steamapps.join("common")).expect("目录");
        std::fs::write(
            steamapps.join("appmanifest_5.acf"),
            acf("5", "Escape", r"D:\Elsewhere"),
        )
        .expect("写");
        assert!(steam_games(&steamapps).is_empty());
    }

    #[test]
    fn duplicate_directories_are_merged_and_names_sorted() {
        let tree = tree("dedupe");
        let a = tree.0.join("A");
        let b = tree.0.join("B");
        std::fs::create_dir_all(&a).expect("目录");
        std::fs::create_dir_all(&b).expect("目录");
        let items = vec![
            FoundGame::direct("Zeta".to_owned(), b.clone(), "1".to_owned()),
            FoundGame::direct("Alpha".to_owned(), a.clone(), "1".to_owned()),
            FoundGame::direct("Zeta again".to_owned(), b.clone(), "2".to_owned()),
            FoundGame::direct("Gone".to_owned(), tree.0.join("gone"), "1".to_owned()),
        ];
        let merged = deduplicate(items);
        assert_eq!(merged.len(), 2, "{merged:?}");
        assert_eq!(merged[0].name, "Alpha", "按名字排序");
        assert_eq!(merged[1].name, "Zeta");
    }

    #[test]
    fn redistributables_are_not_games() {
        for (name, directory) in [
            ("Steamworks Common Redistributables", "Steamworks Shared"),
            ("Proton Experimental", "Proton - Experimental"),
            (
                "Steam Linux Runtime 3.0 (sniper)",
                "SteamLinuxRuntime_sniper",
            ),
        ] {
            assert!(is_redistributable(name, directory), "应跳过 {name}");
        }
        for (name, directory) in [
            ("黑神话：悟空", "BlackMythWukong"),
            ("Risk of Rain 2", "Risk of Rain 2"),
        ] {
            assert!(!is_redistributable(name, directory), "不应跳过 {name}");
        }
    }

    #[test]
    fn gog_and_epic_entries_map_to_direct_games() {
        let gog = gog_games(&[("Cyberpunk 2077".to_owned(), PathBuf::from(r"D:\GOG\CP"))]);
        assert_eq!(gog[0].name, "Cyberpunk 2077");
        assert_eq!(gog[0].launch_mode, "Direct", "GOG 是 DRM-free，直接起 EXE");
        assert!(gog[0].app_id.is_none());
        assert!(gog[0].epic_app_name.is_none());

        let epic = epic_games(&[
            (
                "Sakura".to_owned(),
                Some("Sakura: The Game".to_owned()),
                PathBuf::from(r"D:\Epic\Sakura"),
            ),
            (
                "NoDisplayName".to_owned(),
                None,
                PathBuf::from(r"D:\Epic\X"),
            ),
        ]);
        assert_eq!(epic[0].name, "Sakura: The Game");
        assert_eq!(epic[1].name, "NoDisplayName", "没有显示名时用 AppName");
        assert_eq!(epic[0].launch_mode, "Epic", "Epic 游戏要由 Epic 启动器拉起");
        assert_eq!(epic[0].epic_app_name.as_deref(), Some("Sakura"));
        assert_eq!(epic[1].epic_app_name.as_deref(), Some("NoDisplayName"));
    }

    #[test]
    fn epic_launch_uri_is_built_from_the_app_name() {
        assert_eq!(
            epic_launch_uri("Gingerbread").expect("URI"),
            "com.epicgames.launcher://apps/Gingerbread?action=launch&silent=true"
        );
        assert_eq!(
            epic_launch_uri("  a1_b2-c3.d4  ").expect("去空白"),
            "com.epicgames.launcher://apps/a1_b2-c3.d4?action=launch&silent=true"
        );
        for bad in ["", "   ", "../evil", "a b", "a?b", "a&b"] {
            assert!(epic_launch_uri(bad).is_err(), "应拒绝 {bad:?}");
        }
    }

    #[test]
    fn pair_parser_needs_exactly_one_line() {
        assert_eq!(
            single("\t\"name\"\t\t\"黑神话：悟空\"\n", "name").as_deref(),
            Some("黑神话：悟空")
        );
        assert_eq!(single("\"name\" \"a\"\n\"name\" \"b\"\n", "name"), None);
        assert_eq!(single("\"other\" \"a\"\n", "name"), None);
    }
}
