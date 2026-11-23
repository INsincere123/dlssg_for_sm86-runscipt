//! 内置游戏定位（鸣潮 / 异环）。
//!
//! 这两款不在 Steam/GOG/Epic 上，只能靠注册表卸载项的显示名认出安装目录，再用**已知的
//! 相对路径**核对渲染 EXE 与配置目录。**不读 `启动脚本/启动配置.ini`**（用户确认：图形
//! 入口不用那份 INI）。核对不通过就只报"找到目录但不确认"，不猜渲染程序。
//!
//! 模板取自真机核对（2026-11-23）：
//!
//! | 游戏 | 渲染 EXE（相对游戏目录） | 配置目录 | 官方启动器 | 代理 | 会话 |
//! |---|---|---|---|---|---|
//! | 鸣潮 | `Client\Binaries\Win64\Client-Win64-Shipping.exe` | `<游戏>\Client\Saved\Config\WindowsNoEditor` | 注册表 `DisplayIcon`（与游戏目录不同层） | version.dll | hot-restart:10 |
//! | 异环 | `Client\WindowsNoEditor\HT\Binaries\Win64\HTGame.exe` | `%LOCALAPPDATA%\HT\Saved\Config\Windows` | `<游戏>\NTELauncher.exe` | winmm.dll | first-exit |
//!
//! 注册表认不出时还有一层**浅层查找**（[`search_game_directory`]）：真机鸣潮的卸载项只登记
//! 启动器 `C:\Program Files\Wuthering Waves`，游戏本体在 `C:\Games\Wuthering Waves Game`，
//! 平台记录里没有任何一行指向它。所以按"目录名对上 + 模板里的渲染 EXE 真的存在"在固定磁盘的
//! 前几层目录里再找一遍：名字只用来缩小范围，**核对渲染 EXE 才是证据**，找到多个就报歧义。

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

/// 注册表卸载项的投影（`launcher-win` 负责读，这里只认字段）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UninstallEntry {
    pub display_name: String,
    pub install_location: Option<String>,
    pub install_path: Option<String>,
    pub display_icon: Option<String>,
}

/// 一个已知游戏的模板。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinGame {
    /// 偏好里的稳定 ID（与 PS 一致，便于沿用旧记录）。
    pub id: &'static str,
    pub name: &'static str,
    /// 注册表显示名里可以出现的片段（小写比较）。
    pub display_names: &'static [&'static str],
    /// 渲染 EXE（相对游戏目录）。
    pub render_executable: &'static str,
    /// 官方启动器的入口文件名（我们拉起的那个）。
    pub launcher_file: &'static str,
    /// 启动器真正跑起来的进程名（可能与入口不同：鸣潮入口 `NTELauncher.exe`，
    /// 实际进程是 `NTELauncher\NTEGame.exe`）。
    pub launcher_process: &'static str,
    /// 配置目录（相对游戏目录，或含 `%LOCALAPPDATA%` 的绝对模板）。
    pub config_dir: &'static str,
    /// 配置源（相对启动器根）。
    pub config_source: &'static str,
    /// 代理编号：异环必须 2（`winmm.dll`，ACE 不拦）。
    pub proxy_index: u8,
    /// 会话策略。
    pub session_policy: &'static str,
    /// 游戏目录的文件夹名（浅层查找时用来缩小范围；认到名字还要核对渲染 EXE）。
    pub folder_names: &'static [&'static str],
}

/// 鸣潮。
pub const WUTHERING: BuiltinGame = BuiltinGame {
    id: "wuthering",
    name: "鸣潮",
    display_names: &["鸣潮", "wuthering waves"],
    render_executable: r"Client\Binaries\Win64\Client-Win64-Shipping.exe",
    launcher_file: "launcher.exe",
    launcher_process: "launcher.exe",
    config_dir: r"Client\Saved\Config\WindowsNoEditor",
    config_source: r"配置\鸣潮\Engine.ini",
    proxy_index: 1,
    session_policy: "hot-restart:10",
    folder_names: &["Wuthering Waves Game", "Wuthering Waves"],
};

/// 异环。
pub const NTE: BuiltinGame = BuiltinGame {
    id: "nte",
    name: "异环",
    display_names: &["异环", "neverness to everness", "neverness"],
    render_executable: r"Client\WindowsNoEditor\HT\Binaries\Win64\HTGame.exe",
    launcher_file: "NTELauncher.exe",
    // 真机核对：入口 NTELauncher.exe 起的是 NTELauncher\NTEGame.exe（界面是网页启动器）。
    launcher_process: "NTEGame.exe",
    config_dir: r"%LOCALAPPDATA%\HT\Saved\Config\Windows",
    config_source: r"配置\异环\Engine.ini",
    proxy_index: 2,
    session_policy: "first-exit",
    folder_names: &["Neverness To Everness"],
};

/// 全部内置游戏。
pub fn known_games() -> &'static [BuiltinGame] {
    &[WUTHERING, NTE]
}

/// 定位结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub game: &'static BuiltinGame,
    /// 游戏目录（含渲染 EXE 的那一层）。
    pub directory: PathBuf,
    /// 官方启动器（找到才有；鸣潮的启动器和游戏不在同一层）。
    pub launcher: Option<PathBuf>,
    /// 识别来源说明，界面与日志都用它。
    pub source: String,
}

impl Located {
    /// 配置目录：模板含 `%VAR%` 就展开，否则相对游戏目录。
    pub fn config_directory(&self) -> PathBuf {
        match expand_environment(self.game.config_dir) {
            Some(absolute) => absolute,
            None => self.directory.join(self.game.config_dir),
        }
    }

    /// 配置源（相对启动器根或绝对路径）。
    pub fn config_source(&self, launcher_root: &Path) -> PathBuf {
        let declared = Path::new(self.game.config_source);
        if declared.is_absolute() {
            declared.to_path_buf()
        } else {
            launcher_root.join(declared)
        }
    }

    /// 官方启动器的进程名（等它出现时用这个，不是入口文件名）。
    pub fn launcher_process_name(&self) -> &'static str {
        self.game.launcher_process
    }
}

/// 展开 `%LOCALAPPDATA%` 这类模板；不含 `%` 时返回 `None`。
fn expand_environment(template: &str) -> Option<PathBuf> {
    if !template.contains('%') {
        return None;
    }
    let mut result = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('%') {
        result.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find('%')?;
        let name = &after[..end];
        result.push_str(&std::env::var(name).ok()?);
        rest = &after[end + 1..];
    }
    result.push_str(rest);
    Some(PathBuf::from(result.replace('/', "\\")))
}

/// 从注册表卸载项里认出内置游戏。
///
/// 候选目录按顺序试：`InstallLocation` → `InstallPath` → `DisplayIcon` 的父目录，
/// 每个再向下试一层子目录（鸣潮的安装器把游戏装在 `<InstallPath>\Wuthering Waves Game`）。
/// **只有渲染 EXE 真的存在才算识别成功。**
pub fn locate(entries: &[UninstallEntry]) -> Vec<Located> {
    let mut found: Vec<Located> = Vec::new();
    for game in known_games() {
        if found.iter().any(|item| item.game.id == game.id) {
            continue;
        }
        let Some(entry) = entries.iter().find(|entry| entry_matches(game, entry)) else {
            continue;
        };
        for base in candidate_directories(entry) {
            if let Some(directory) = game_directory(game, &base) {
                let launcher = launcher_path(game, Some(entry), &directory);
                found.push(Located {
                    game,
                    directory,
                    launcher,
                    source: format!("注册表卸载项「{}」", entry.display_name),
                });
                break;
            }
        }
    }
    found
}

/// 一个卸载项是不是这款内置游戏（按显示名片段，大小写不敏感）。
pub fn entry_matches(game: &BuiltinGame, entry: &UninstallEntry) -> bool {
    let name = entry.display_name.to_lowercase();
    game.display_names
        .iter()
        .any(|fragment| name.contains(fragment))
}

/// 浅层查找的预算。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchLimits {
    /// 从根目录往下找几层（`C:\Games\Wuthering Waves Game` 是第 2 层）。
    pub max_depth: usize,
    /// 最多读多少个目录；超了就标 `truncated`，不静默截断。
    pub max_directories: usize,
}

impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            max_depth: 3,
            max_directories: 4_000,
        }
    }
}

/// 一次浅层查找的结论。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchOutcome {
    /// 目录名对上、渲染 EXE 也在的目录（升序、去重）。
    pub directories: Vec<PathBuf>,
    /// 目录名对上了，但里面没有渲染 EXE（安装不完整 / 只剩启动器）。
    pub rejected: Vec<PathBuf>,
    /// 实际读过的目录数。
    pub visited: usize,
    /// 达到目录上限，结论可能不完整。
    pub truncated: bool,
}

impl SearchOutcome {
    /// 唯一可信的游戏目录；零个或多个都返回 `None`（不猜）。
    pub fn unique_directory(&self) -> Option<&Path> {
        match self.directories.as_slice() {
            [only] => Some(only),
            _ => None,
        }
    }
}

/// 浅层查找时跳过的目录名：系统目录与不可能装游戏的用户目录。
const SEARCH_SKIP: &[&str] = &[
    "windows",
    "windows.old",
    "$recycle.bin",
    "system volume information",
    "recovery",
    "perflogs",
    "msocache",
    "documents and settings",
    "config.msi",
    "users",
    "programdata",
    "node_modules",
    ".git",
    "efi",
    "boot",
    "inetpub",
];

/// 在给定根目录（一般是固定磁盘根）的前几层里找这款内置游戏的安装目录。
///
/// 用法与边界：只在注册表认不出游戏目录时才调用；`roots` 由调用方给固定磁盘根。
/// 命中条件是**目录名等于模板里的文件夹名**且 `<目录>\<render_executable>` 真的存在——
/// 名字只用来缩小范围，核对渲染 EXE 才算证据。同名但缺渲染 EXE 的目录收进
/// `rejected`（不当作候选），找到多个候选也不替用户挑。
pub fn search_game_directory(
    game: &BuiltinGame,
    roots: &[PathBuf],
    limits: SearchLimits,
) -> SearchOutcome {
    let mut outcome = SearchOutcome::default();
    let render = Path::new(game.render_executable);
    let known_names: Vec<String> = game
        .folder_names
        .iter()
        .map(|name| name.to_lowercase())
        .collect();
    let mut pending: VecDeque<(PathBuf, usize)> = VecDeque::new();
    for root in roots {
        pending.push_back((root.clone(), 0));
    }
    while let Some((directory, depth)) = pending.pop_front() {
        if outcome.visited >= limits.max_directories {
            outcome.truncated = true;
            break;
        }
        outcome.visited += 1;
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            // file_type() 不跟随链接：符号链接/junction 直接跳过，避免绕圈和重复计数。
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if !kind.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if SEARCH_SKIP.contains(&name.as_str()) {
                continue;
            }
            let path = entry.path();
            if known_names.contains(&name) {
                if path.join(render).is_file() {
                    outcome.directories.push(path);
                } else {
                    outcome.rejected.push(path);
                }
                // 游戏目录不会套一层同名目录，别把里面的副本算成第二份。
                continue;
            }
            if depth + 1 < limits.max_depth {
                pending.push_back((path, depth + 1));
            }
        }
    }
    sort_unique(&mut outcome.directories);
    sort_unique(&mut outcome.rejected);
    outcome
}

fn sort_unique(paths: &mut Vec<PathBuf>) {
    paths.sort_by_key(|path| printable(path));
    paths.dedup_by(|left, right| printable(left) == printable(right));
}

fn printable(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

/// 从一个卸载项推出可能的安装目录。
fn candidate_directories(entry: &UninstallEntry) -> Vec<PathBuf> {
    let mut bases: Vec<PathBuf> = Vec::new();
    for value in [
        entry.install_location.as_deref(),
        entry.install_path.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            bases.push(PathBuf::from(trimmed));
        }
    }
    if let Some(parent) =
        display_icon_path(entry).and_then(|path| path.parent().map(Path::to_path_buf))
    {
        bases.push(parent);
    }
    bases
}

/// 把 `DisplayIcon` 里的 `"C:\Game\launcher.exe",0` 拆成路径。
fn display_icon_path(entry: &UninstallEntry) -> Option<PathBuf> {
    let icon = entry.display_icon.as_deref()?.trim();
    let cleaned = icon
        .trim_start_matches('"')
        .split('"')
        .next()
        .unwrap_or("")
        .split(',')
        .next()
        .unwrap_or("")
        .trim();
    (!cleaned.is_empty()).then(|| PathBuf::from(cleaned))
}

/// 在候选目录里找"含渲染 EXE 的那一层"。
fn game_directory(game: &BuiltinGame, base: &Path) -> Option<PathBuf> {
    let render = Path::new(game.render_executable);
    let mut candidates: Vec<PathBuf> = vec![base.to_path_buf()];
    if let Ok(entries) = std::fs::read_dir(base) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !path.is_symlink() {
                candidates.push(path);
            }
        }
    }
    candidates
        .into_iter()
        .find(|directory| directory.join(render).is_file())
}

/// 官方启动器：先在游戏目录里找，找不到就用卸载项的 `DisplayIcon`。
///
/// 注册表认出来和浅层查找用的是同一套规则（鸣潮的启动器和游戏不在同一层），
/// 没有卸载项时只认游戏目录里的入口。
pub fn launcher_path(
    game: &BuiltinGame,
    entry: Option<&UninstallEntry>,
    directory: &Path,
) -> Option<PathBuf> {
    let relative = directory.join(game.launcher_file);
    if relative.is_file() {
        return Some(relative);
    }
    let icon = display_icon_path(entry?)?;
    if icon
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case(game.launcher_file))
        .unwrap_or(false)
        && icon.is_file()
    {
        return Some(icon);
    }
    None
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
            std::env::temp_dir().join(format!("dlssg-builtin-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("临时目录");
        Tree(root)
    }

    fn write_file(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("目录");
        }
        std::fs::write(path, b"MZ").expect("写");
    }

    fn fake_wuthering(root: &Path) -> PathBuf {
        let game = root.join("Wuthering Waves Game");
        write_file(&game.join(r"Client\Binaries\Win64\Client-Win64-Shipping.exe"));
        game
    }

    fn fake_nte(root: &Path) -> PathBuf {
        write_file(&root.join(r"Client\WindowsNoEditor\HT\Binaries\Win64\HTGame.exe"));
        write_file(&root.join("NTELauncher.exe"));
        root.to_path_buf()
    }

    #[test]
    fn locate_finds_the_directory_holding_the_renderer() {
        let tree = tree("locate");
        let game = fake_wuthering(&tree.0);
        let entry = UninstallEntry {
            display_name: "鸣潮".to_owned(),
            install_path: Some(tree.0.display().to_string()),
            ..UninstallEntry::default()
        };
        let located = locate(&[entry]);
        assert_eq!(located.len(), 1, "{located:?}");
        assert_eq!(located[0].game.id, "wuthering");
        assert_eq!(located[0].directory, game);
        assert!(located[0].source.contains("鸣潮"));
    }

    #[test]
    fn wuthering_launcher_comes_from_the_display_icon() {
        let tree = tree("wuthering-launcher");
        let game = fake_wuthering(&tree.0);
        let launcher_dir = tree.0.join("Launcher");
        write_file(&launcher_dir.join("launcher.exe"));
        let entry = UninstallEntry {
            display_name: "Wuthering Waves".to_owned(),
            install_path: Some(tree.0.display().to_string()),
            display_icon: Some(format!("\"{}\\launcher.exe\",0", launcher_dir.display())),
            ..UninstallEntry::default()
        };
        let located = locate(&[entry]);
        assert_eq!(located.len(), 1);
        assert_eq!(located[0].directory, game);
        assert_eq!(
            located[0].launcher.as_deref(),
            Some(launcher_dir.join("launcher.exe").as_path()),
            "启动器不在游戏目录里"
        );
    }

    #[test]
    fn nte_launcher_is_inside_the_game_directory() {
        let tree = tree("nte");
        let game = fake_nte(&tree.0);
        let entry = UninstallEntry {
            display_name: "异环".to_owned(),
            install_location: Some(game.display().to_string()),
            ..UninstallEntry::default()
        };
        let located = locate(&[entry]);
        assert_eq!(located.len(), 1);
        assert_eq!(located[0].game.id, "nte");
        assert_eq!(located[0].directory, game);
        assert_eq!(
            located[0].launcher.as_deref(),
            Some(game.join("NTELauncher.exe").as_path())
        );
        assert_eq!(
            located[0].launcher_process_name(),
            "NTEGame.exe",
            "等的是真实进程名，不是入口文件名"
        );
    }

    #[test]
    fn missing_renderer_means_not_located() {
        let tree = tree("missing");
        std::fs::create_dir_all(tree.0.join("Client")).expect("目录");
        let entry = UninstallEntry {
            display_name: "异环".to_owned(),
            install_location: Some(tree.0.display().to_string()),
            ..UninstallEntry::default()
        };
        assert!(locate(&[entry]).is_empty(), "不能只凭目录名认游戏");
    }

    #[test]
    fn unknown_display_names_are_ignored() {
        let entry = UninstallEntry {
            display_name: "Some Other Game".to_owned(),
            install_location: Some(r"C:\Games\X".to_owned()),
            ..UninstallEntry::default()
        };
        assert!(locate(&[entry]).is_empty());
    }

    #[test]
    fn config_dir_expands_environment_and_joins_relative_ones() {
        let tree = tree("config");
        let game = fake_nte(&tree.0);
        let nte = Located {
            game: &NTE,
            directory: game.clone(),
            launcher: None,
            source: "test".to_owned(),
        };
        let local = std::env::var("LOCALAPPDATA").expect("LOCALAPPDATA");
        assert_eq!(
            nte.config_directory(),
            PathBuf::from(local).join(r"HT\Saved\Config\Windows")
        );

        let wuthering = Located {
            game: &WUTHERING,
            directory: tree.0.join("Wuthering Waves Game"),
            launcher: None,
            source: "test".to_owned(),
        };
        assert_eq!(
            wuthering.config_directory(),
            tree.0
                .join("Wuthering Waves Game")
                .join(r"Client\Saved\Config\WindowsNoEditor")
        );
    }

    #[test]
    fn config_source_is_relative_to_the_launcher_root() {
        let located = Located {
            game: &NTE,
            directory: PathBuf::from(r"C:\Games\NTE"),
            launcher: None,
            source: "test".to_owned(),
        };
        assert_eq!(
            located.config_source(Path::new(r"D:\launcher")),
            PathBuf::from(r"D:\launcher\配置\异环\Engine.ini")
        );
    }

    #[test]
    fn templates_keep_the_verified_differences() {
        assert_eq!(NTE.proxy_index, 2, "异环必须 winmm.dll");
        assert_eq!(WUTHERING.proxy_index, 1);
        assert_eq!(NTE.session_policy, "first-exit");
        assert_eq!(WUTHERING.session_policy, "hot-restart:10");
        assert_eq!(NTE.config_source, r"配置\异环\Engine.ini");
        assert_eq!(WUTHERING.config_source, r"配置\鸣潮\Engine.ini");
        assert!(NTE.config_dir.starts_with("%LOCALAPPDATA%"));
        assert!(!WUTHERING.config_dir.contains('%'));
        assert_eq!(
            WUTHERING.folder_names,
            &["Wuthering Waves Game", "Wuthering Waves"]
        );
        assert_eq!(NTE.folder_names, &["Neverness To Everness"]);
    }

    #[test]
    fn display_names_match_the_installed_entries() {
        let wuthering = UninstallEntry {
            display_name: "鸣潮".to_owned(),
            ..UninstallEntry::default()
        };
        assert!(entry_matches(&WUTHERING, &wuthering));
        assert!(!entry_matches(&NTE, &wuthering));
        let english = UninstallEntry {
            display_name: "Wuthering Waves".to_owned(),
            ..UninstallEntry::default()
        };
        assert!(entry_matches(&WUTHERING, &english));
    }

    /// 真机形态：注册表只登记启动器（`C:\Program Files\Wuthering Waves`），
    /// 游戏本体在 `C:\Games\Wuthering Waves Game`。
    #[test]
    fn search_finds_the_game_installed_outside_the_registry_root() {
        let tree = tree("search-found");
        // 注册表指向的启动器根：同名却没有游戏本体。
        write_file(&tree.0.join(r"Program Files\Wuthering Waves\launcher.exe"));
        std::fs::create_dir_all(
            tree.0
                .join(r"Program Files\Wuthering Waves\Wuthering Waves Game"),
        )
        .expect("目录");
        let game = fake_wuthering(&tree.0.join("Games"));

        let outcome = search_game_directory(
            &WUTHERING,
            std::slice::from_ref(&tree.0),
            SearchLimits::default(),
        );
        assert_eq!(outcome.directories, vec![game.clone()], "{outcome:?}");
        assert_eq!(outcome.unique_directory(), Some(game.as_path()));
        assert!(
            outcome
                .rejected
                .iter()
                .any(|path| path.ends_with(r"Program Files\Wuthering Waves")),
            "启动器根要进 rejected，不能当候选：{:?}",
            outcome.rejected
        );
        assert!(!outcome.truncated);
    }

    #[test]
    fn search_ignores_directories_without_the_renderer() {
        let tree = tree("search-empty");
        std::fs::create_dir_all(tree.0.join("Wuthering Waves Game")).expect("目录");
        let outcome = search_game_directory(
            &WUTHERING,
            std::slice::from_ref(&tree.0),
            SearchLimits::default(),
        );
        assert!(outcome.directories.is_empty(), "{outcome:?}");
        assert_eq!(outcome.rejected.len(), 1);
        assert_eq!(outcome.unique_directory(), None);
    }

    #[test]
    fn search_refuses_to_pick_between_two_installs() {
        let tree = tree("search-two");
        let first = fake_wuthering(&tree.0.join("Games"));
        let second = fake_wuthering(&tree.0.join(r"Games2\Nested"));
        let outcome = search_game_directory(
            &WUTHERING,
            std::slice::from_ref(&tree.0),
            SearchLimits::default(),
        );
        assert_eq!(outcome.directories.len(), 2, "{outcome:?}");
        assert!(outcome.directories.contains(&first));
        assert!(outcome.directories.contains(&second));
        assert_eq!(outcome.unique_directory(), None, "多个候选不能替用户挑");
    }

    #[test]
    fn search_stops_at_the_depth_limit_and_says_so() {
        let tree = tree("search-depth");
        let deep = fake_wuthering(&tree.0.join(r"a\b\c\d"));
        let limits = SearchLimits {
            max_depth: 3,
            max_directories: 4_000,
        };
        let outcome = search_game_directory(&WUTHERING, std::slice::from_ref(&tree.0), limits);
        assert!(
            !outcome.directories.contains(&deep),
            "第 5 层的目录不在 max_depth=3 里：{outcome:?}"
        );

        let shallow = fake_wuthering(&tree.0.join(r"a\b"));
        let outcome = search_game_directory(&WUTHERING, std::slice::from_ref(&tree.0), limits);
        assert_eq!(outcome.directories, vec![shallow], "{outcome:?}");
    }

    #[test]
    fn search_reports_a_directory_budget_overrun() {
        let tree = tree("search-budget");
        fake_wuthering(&tree.0.join("Games"));
        let limits = SearchLimits {
            max_depth: 3,
            max_directories: 1,
        };
        let outcome = search_game_directory(&WUTHERING, std::slice::from_ref(&tree.0), limits);
        assert!(outcome.truncated, "{outcome:?}");
        assert!(outcome.directories.is_empty());
    }

    #[test]
    fn launcher_path_can_work_without_an_entry() {
        let tree = tree("launcher-none");
        let game = fake_nte(&tree.0);
        assert_eq!(
            launcher_path(&NTE, None, &game),
            Some(game.join("NTELauncher.exe"))
        );
        std::fs::remove_file(game.join("NTELauncher.exe")).expect("删");
        assert_eq!(launcher_path(&NTE, None, &game), None);
    }
}
