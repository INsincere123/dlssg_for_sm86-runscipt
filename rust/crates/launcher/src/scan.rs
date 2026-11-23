//! 平台扫描：注册表与平台清单 → 已安装游戏 → 确认渲染 EXE → 可直接添加的配置。
//!
//! 用户口径（2026-11-23）：**只扫平台，不翻全盘**；能唯一确认渲染程序的直接加进来，
//! 少让用户点。所以顺序是：
//!
//! 1. 内置定位（鸣潮 / 异环）：注册表卸载项 + 已知相对路径，命中即用模板
//! 2. 内置游戏的浅层兜底：真机鸣潮的卸载项只有启动器（`C:\Program Files\Wuthering Waves`），
//!    游戏本体在 `C:\Games\Wuthering Waves Game`——平台记录里没有这一行。于是按"目录名对上
//!    模板里的文件夹名 + 那个目录里真的有渲染 EXE"在固定磁盘前几层再找一遍（有上限、
//!    跳过系统目录）；找到多个就报歧义，绝不替用户挑
//! 3. Steam：每个库的 `appmanifest_*.acf`（AppID / 名称 / installdir）
//! 4. GOG：注册表逐游戏子键
//! 5. Epic：`LauncherInstalled.dat` + `.item` 里的显示名
//!
//! 对平台给出的每个目录，再用 `discovery` 打分找渲染 EXE：**唯一可信（≥60 分且领先）**
//! 就直接成为可添加的配置，否则进"需要确认"，绝不在多个候选里随便挑一个。
//!
//! 已经在偏好里的游戏（按 ID 或目录）直接跳过，不重复添加。

use std::path::{Path, PathBuf};

use launcher_core::builtin::{self, Located};
use launcher_core::discovery::{self, DiscoveryLimits};
use launcher_core::platform::{self, FoundGame};
use launcher_core::preferences::GameProfile;

/// 单个游戏的扫描预算：只在一个游戏目录里找，不需要全盘的 12000。
const LIMITS: DiscoveryLimits = DiscoveryLimits {
    max_directories: 4_000,
    max_depth: 10,
};

/// 内置游戏浅层查找的预算：真机（C/D/E 三个固定磁盘）只读到 317 个目录、16 毫秒左右；
/// 上限只是防止别人的机器上目录特别多时把扫描拖住。
const SEARCH_LIMITS: builtin::SearchLimits = builtin::SearchLimits {
    max_depth: 3,
    max_directories: 4_000,
};

/// 可以直接添加的游戏。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyGame {
    pub profile: GameProfile,
    /// 来源说明（平台名/注册表），写进提示。
    pub source: String,
}

/// 找到了目录但渲染 EXE 不唯一。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmbiguousGame {
    pub name: String,
    pub directory: PathBuf,
    pub source: String,
    pub reason: String,
}

/// 一次扫描的结论。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanResult {
    pub ready: Vec<ReadyGame>,
    pub ambiguous: Vec<AmbiguousGame>,
    /// 平台层面的说明（读了哪些来源、有哪些跳过）。
    pub notes: Vec<String>,
}

impl ScanResult {
    pub fn summary(&self) -> String {
        let mut text = format!(
            "扫描完成：可添加 {} 个，需要确认 {} 个",
            self.ready.len(),
            self.ambiguous.len()
        );
        if !self.notes.is_empty() {
            text.push_str(&format!("（{}）", self.notes.join("；")));
        }
        text
    }
}

fn same_directory(left: &Path, right: &Path) -> bool {
    let key = |path: &Path| {
        path.to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    };
    key(left) == key(right)
}

/// 扫描全部来源。
pub fn scan(launcher_root: &Path, existing: &[(String, PathBuf)]) -> ScanResult {
    let mut result = ScanResult::default();
    let known = |id: &str, directory: &Path| -> bool {
        existing.iter().any(|(existing_id, existing_dir)| {
            existing_id == id || same_directory(existing_dir, directory)
        })
    };

    // 1) 内置定位：注册表卸载项；注册表只登记启动器时，再按目录名浅层找一遍
    let entries = launcher_win::registry::uninstall_entries();
    let mut located = builtin::locate(&entries);
    let drives = launcher_win::drives::fixed_drive_roots();
    for game in builtin::known_games() {
        if located.iter().any(|item| item.game.id == game.id) {
            continue;
        }
        let entry = entries
            .iter()
            .find(|entry| builtin::entry_matches(game, entry));
        let outcome = builtin::search_game_directory(game, &drives, SEARCH_LIMITS);
        if let Some(directory) = outcome.unique_directory() {
            located.push(Located {
                game,
                directory: directory.to_path_buf(),
                launcher: builtin::launcher_path(game, entry, directory),
                source: "浅层查找：目录名 + 渲染程序核对".to_owned(),
            });
            continue;
        }
        if outcome.directories.len() > 1 {
            result.ambiguous.push(AmbiguousGame {
                name: game.name.to_owned(),
                directory: outcome.directories[0].clone(),
                source: "浅层查找".to_owned(),
                reason: format!(
                    "找到 {} 个同名游戏目录，需要手动确认用哪个：{}",
                    outcome.directories.len(),
                    outcome
                        .directories
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join("；")
                ),
            });
            continue;
        }
        // 没找到：把现状写清楚，让用户手动「添加游戏」。
        let mut note = if entry.is_some() {
            format!(
                "注册表里的「{}」没给出能核对通过的安装目录（多半只登记了启动器）；浅层查找也没找到 {}，需要手动「添加游戏」",
                game.name, game.render_executable
            )
        } else {
            format!(
                "没在注册表里认出「{}」的安装目录，需要的话手动「添加游戏」",
                game.name
            )
        };
        if let Some(rejected) = outcome.rejected.first() {
            note.push_str(&format!("；同名目录里没有渲染程序：{}", rejected.display()));
        }
        if outcome.truncated {
            note.push_str("；浅层查找达到目录上限，可能没找全");
        }
        result.notes.push(note);
    }
    for item in located {
        // 同目录的旧记录（例如以前以 direct-… 加过的鸣潮）也算已存在，避免加出两份。
        let duplicate = existing.iter().any(|(existing_id, existing_dir)| {
            existing_id == item.game.id || same_directory(existing_dir, &item.directory)
        });
        if duplicate {
            continue;
        }
        match profile_from_builtin(launcher_root, &item) {
            Ok(profile) => result.ready.push(ReadyGame {
                profile,
                source: item.source.clone(),
            }),
            Err(reason) => result.ambiguous.push(AmbiguousGame {
                name: item.game.name.to_owned(),
                directory: item.directory.clone(),
                source: item.source.clone(),
                reason,
            }),
        }
    }

    // 2) 平台清单
    let mut found: Vec<FoundGame> = Vec::new();
    let mut steam_libraries = 0usize;
    for steamapps in launcher_win::registry::steam_library_steamapps() {
        steam_libraries += 1;
        found.extend(platform::steam_games(&steamapps));
    }
    if steam_libraries == 0 {
        result.notes.push("没读到 Steam 库位置".to_owned());
    }
    let gog = launcher_win::registry::gog_games();
    if gog.is_empty() {
        result.notes.push("没有 GOG 记录".to_owned());
    }
    found.extend(platform::gog_games(&gog));
    let epic = launcher_win::registry::epic_games();
    if epic.is_empty() {
        result.notes.push("没有 Epic 记录".to_owned());
    }
    found.extend(platform::epic_games(&epic));

    let found = platform::deduplicate(found);
    if found.is_empty() {
        result.notes.push("平台清单里没有已安装的游戏".to_owned());
    }

    // 3) 逐个确认渲染 EXE
    for game in found {
        if known("", &game.directory) {
            continue;
        }
        let id = profile_id(&game);
        if known(&id, &game.directory) {
            continue;
        }
        match discovery::renderer_candidates(&game.directory, None, LIMITS, &|| false) {
            Ok(outcome) => match discovery::trusted_renderer(&outcome.candidates) {
                Some(candidate) => {
                    // 只有"看得出是虚幻打包结构"或"游戏自带 DLSS-G 组件"才自动加：
                    // 其它（Unity / 老引擎）加进来也用不上帧生成，交给用户确认。
                    let evidence = auto_add_evidence(candidate);
                    if evidence.is_none() {
                        result.ambiguous.push(AmbiguousGame {
                            name: game.name.clone(),
                            directory: game.directory.clone(),
                            source: game.source.clone(),
                            reason: "没看到虚幻打包结构或自带 DLSS-G 组件".to_owned(),
                        });
                        continue;
                    }
                    let relative = candidate
                        .path
                        .strip_prefix(&game.directory)
                        .map(|path| path.to_string_lossy().replace('/', "\\"))
                        .unwrap_or_else(|_| candidate.relative.clone());
                    let mut profile = GameProfile::new(&id, &game.name, game.directory.clone());
                    profile.launch_mode = game.launch_mode.clone();
                    profile.render_executable = Some(relative);
                    profile.proxy_index =
                        crate::settings_form::SettingsForm::recommended_proxy(&id, &game.directory);
                    if let Some(app_id) = game.app_id.clone() {
                        match launcher_core::steam::launch_arguments(&app_id) {
                            Ok(arguments) => {
                                profile.app_id = Some(app_id);
                                profile.launch_arguments = Some(arguments);
                                // Steam 游戏必须用 steam.exe 启动；直接用游戏 EXE 会绕开 Steam。
                                if let Some(steam) =
                                    launcher_win::registry::steam_install_directory()
                                {
                                    let executable = steam.join("Steam.exe");
                                    if executable.is_file() {
                                        profile.launch_executable =
                                            Some(executable.display().to_string());
                                        profile.launcher_process_name =
                                            Some("steam.exe".to_owned());
                                    }
                                }
                            }
                            Err(_) => continue,
                        }
                    }
                    result.ready.push(ReadyGame {
                        profile,
                        source: game.source,
                    });
                }
                None => result.ambiguous.push(AmbiguousGame {
                    name: game.name.clone(),
                    directory: game.directory.clone(),
                    source: game.source.clone(),
                    reason: if outcome.candidates.is_empty() {
                        "没找到 x64 游戏程序".to_owned()
                    } else {
                        format!(
                            "有 {} 个候选但没有唯一可信项（最高 {} 分）",
                            outcome.candidates.len(),
                            outcome.candidates[0].score
                        )
                    },
                }),
            },
            Err(error) => result.ambiguous.push(AmbiguousGame {
                name: game.name.clone(),
                directory: game.directory.clone(),
                source: game.source.clone(),
                reason: format!("扫描失败：{error}"),
            }),
        }
    }

    result
}

/// 自动添加的证据：是 UE 打包本体（`-Win64-Shipping.exe`），或同目录就有 DLSS-G 组件。
///
/// 返回一句人话说明（写进提示），不是证据时返回 `None`。
fn auto_add_evidence(candidate: &launcher_core::discovery::RendererCandidate) -> Option<String> {
    let name = candidate
        .path
        .file_name()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if name.ends_with("-win64-shipping.exe") {
        return Some("UE 打包本体".to_owned());
    }
    let directory = candidate.path.parent()?;
    for entry in std::fs::read_dir(directory).ok()?.flatten() {
        let file = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if file == "nvngx_dlssg.dll" || file == "sl.dlss_g.dll" {
            return Some(format!("同目录有 {file}"));
        }
    }
    None
}

/// Steam 游戏用 `steam-<AppID>`，其它用目录推导的稳定 ID。
fn profile_id(game: &FoundGame) -> String {
    match game.app_id.as_deref() {
        Some(app_id) => format!("steam-{app_id}"),
        None => launcher_core::direct::new_direct_id(&game.directory.to_string_lossy()),
    }
}

/// 用内置模板生成一份完整配置（渲染 EXE / 配置目录 / 启动器 / 代理 / 会话）。
pub fn profile_from_builtin(launcher_root: &Path, item: &Located) -> Result<GameProfile, String> {
    let directory = item.directory.clone();
    let render = Path::new(item.game.render_executable);
    if !directory.join(render).is_file() {
        return Err(format!("缺少渲染程序：{}", render.display()));
    }
    let mut profile = GameProfile::new(item.game.id, item.game.name, directory.clone());
    profile.launch_mode = "Direct".to_owned();
    profile.render_executable = Some(item.game.render_executable.to_owned());
    profile.proxy_index = item.game.proxy_index;
    profile.session_policy = Some(item.game.session_policy.to_owned());
    profile.config_dir = Some(item.config_directory().display().to_string());
    // 只有配置源真的存在才写进档案：写一个不存在的路径会让 plan 直接失败（曾经因此
    // 让"扫描出来的异环"完全启动不了）。不存在时留空，由引擎按约定路径去找。
    let config_source = item.config_source(launcher_root);
    profile.config_source = config_source
        .is_file()
        .then(|| config_source.display().to_string());
    if let Some(launcher) = item.launcher.as_ref() {
        profile.launch_executable = Some(launcher.display().to_string());
        // 等的是真正的启动器进程名（鸣潮入口 NTELauncher.exe → 进程 NTEGame.exe）。
        profile.launcher_process_name = Some(item.launcher_process_name().to_owned());
    }
    // 引擎版本交给后台识别；识别不出来时用户在「画质」里手动校准。
    profile.engine_host = "none".to_owned();
    Ok(profile)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_compare_ignores_case_and_trailing_separator() {
        assert!(same_directory(
            Path::new(r"D:\Games\A"),
            Path::new(r"d:\games\a\")
        ));
        assert!(!same_directory(
            Path::new(r"D:\Games\A"),
            Path::new(r"D:\Games\B")
        ));
    }

    #[test]
    fn steam_ids_use_the_app_id() {
        let game = FoundGame {
            name: "黑神话：悟空".to_owned(),
            directory: PathBuf::from(r"D:\Steam\steamapps\common\BlackMythWukong"),
            source: "Steam".to_owned(),
            app_id: Some("2358720".to_owned()),
            launch_mode: "Steam".to_owned(),
        };
        assert_eq!(profile_id(&game), "steam-2358720");
    }

    #[test]
    fn direct_ids_are_stable_and_valid() {
        let game = FoundGame {
            name: "PBZ".to_owned(),
            directory: PathBuf::from(r"C:\Games\PBZ"),
            source: "GOG".to_owned(),
            app_id: None,
            launch_mode: "Direct".to_owned(),
        };
        let id = profile_id(&game);
        assert_eq!(id, profile_id(&game), "同一目录必须得到同一 ID");
        assert!(
            launcher_core::direct::validate_direct_id(&id).is_ok(),
            "{id}"
        );
    }

    #[test]
    fn builtin_profile_carries_the_verified_fields() {
        let root = std::env::temp_dir().join(format!("dlssg-scan-{}-builtin", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let game = root.join("Neverness To Everness");
        let render = game.join(r"Client\WindowsNoEditor\HT\Binaries\Win64");
        std::fs::create_dir_all(&render).expect("目录");
        std::fs::write(render.join("HTGame.exe"), b"MZ").expect("写");
        std::fs::write(game.join("NTELauncher.exe"), b"MZ").expect("写");

        let item = Located {
            game: &builtin::NTE,
            directory: game.clone(),
            launcher: Some(game.join("NTELauncher.exe")),
            source: "注册表".to_owned(),
        };
        let profile = profile_from_builtin(Path::new(r"D:\root"), &item).expect("应能生成");
        assert_eq!(profile.id, "nte");
        assert_eq!(profile.proxy_index, 2, "异环必须 winmm.dll");
        assert_eq!(profile.session_policy.as_deref(), Some("first-exit"));
        assert!(
            profile.config_source.is_none(),
            "配置源不存在时不能写进档案（否则 plan 会直接失败）：{:?}",
            profile.config_source
        );
        assert_eq!(
            profile.render_executable.as_deref(),
            Some(builtin::NTE.render_executable)
        );
        assert!(
            profile
                .config_dir
                .as_deref()
                .unwrap_or_default()
                .ends_with(r"HT\Saved\Config\Windows")
        );
        assert!(profile.launch_executable.is_some(), "启动器路径要写进去");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_renderer_is_refused() {
        let root = std::env::temp_dir().join(format!("dlssg-scan-{}-missing", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("目录");
        let item = Located {
            game: &builtin::NTE,
            directory: root.clone(),
            launcher: None,
            source: "注册表".to_owned(),
        };
        let error = profile_from_builtin(Path::new(r"D:\root"), &item).expect_err("应拒绝");
        assert!(error.contains("缺少渲染程序"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 真机探针：`cargo test -p launcher --bin DlssgLauncher scan -- --ignored --nocapture probe_real_scan`
    ///
    /// 读本机的注册表与平台清单，打印扫描结论（不写偏好）。
    #[test]
    #[ignore = "真机检查用：读本机 Steam/GOG/Epic 记录与注册表"]
    fn probe_real_scan() {
        let root = Path::new(r"D:\Games\dlssg_for_sm86");
        let result = scan(root, &[]);
        println!("{}", result.summary());
        for ready in &result.ready {
            println!(
                "可添加：{} → {}（{}；渲染 {}；启动器 {}；配置目录 {}；会话 {}；代理 {}）",
                ready.profile.name,
                ready.profile.directory.display(),
                ready.source,
                ready.profile.render_executable.as_deref().unwrap_or("?"),
                ready
                    .profile
                    .launch_executable
                    .as_deref()
                    .unwrap_or("（未识别）"),
                ready.profile.config_dir.as_deref().unwrap_or("（空）"),
                ready.profile.session_policy.as_deref().unwrap_or("（空）"),
                ready.profile.proxy_index
            );
        }
        for item in &result.ambiguous {
            println!(
                "待确认：{} → {}（{}；{}）",
                item.name,
                item.directory.display(),
                item.source,
                item.reason
            );
        }
        for note in &result.notes {
            println!("说明：{note}");
        }
        // 顺便量一下浅层兜底的代价：它是注册表认不出内置游戏时唯一会读到磁盘外层的步骤。
        let drives = launcher_win::drives::fixed_drive_roots();
        let started = std::time::Instant::now();
        let outcome = builtin::search_game_directory(&builtin::WUTHERING, &drives, SEARCH_LIMITS);
        println!(
            "浅层查找：固定磁盘 {} 个（{}），读过 {} 个目录，用时 {:?}，截断 {}，命中 {:?}，排除 {:?}",
            drives.len(),
            drives
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("、"),
            outcome.visited,
            started.elapsed(),
            outcome.truncated,
            outcome.directories,
            outcome.rejected
        );
    }

    #[test]
    fn summary_counts_both_buckets() {
        let result = ScanResult {
            ready: Vec::new(),
            ambiguous: vec![AmbiguousGame {
                name: "X".to_owned(),
                directory: PathBuf::from(r"D:\X"),
                source: "Steam".to_owned(),
                reason: "没找到 x64 游戏程序".to_owned(),
            }],
            notes: vec!["没有 GOG 记录".to_owned()],
        };
        let text = result.summary();
        assert!(text.contains("可添加 0 个"), "{text}");
        assert!(text.contains("需要确认 1 个"), "{text}");
        assert!(text.contains("没有 GOG 记录"), "{text}");
    }
}
