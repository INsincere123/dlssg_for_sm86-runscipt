//! M3-01：Steam 库解析、安装目录查找、AppID 识别（对齐 `SteamLauncher.ps1`）。

use std::path::{Path, PathBuf};

use launcher_core::path::PathContext;
use launcher_core::steam::{
    find_game_directory, launch_arguments, library_directories, resolve_app_id,
};

const APP_ID: &str = "4356430";

struct Steam {
    root: PathBuf,
    executable: PathBuf,
}

impl Drop for Steam {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Steam {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("dlssg-steam-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("Steam 根");
        let executable = root.join("steam.exe");
        std::fs::write(&executable, "stub").expect("steam.exe");
        std::fs::create_dir_all(root.join("steamapps")).expect("steamapps");
        Self { root, executable }
    }

    fn library(&self, name: &str) -> PathBuf {
        let library = self.root.join(name);
        std::fs::create_dir_all(library.join("steamapps")).expect("库目录");
        library
    }

    /// 写 `libraryfolders.vdf`（路径用 `\\` 转义，和真实文件一致）。
    fn library_folders(&self, libraries: &[&Path]) {
        let mut text = String::from("\"libraryfolders\"\r\n{\r\n");
        for (index, library) in libraries.iter().enumerate() {
            text.push_str(&format!("\t\"{index}\"\r\n\t{{\r\n"));
            text.push_str(&format!(
                "\t\t\"path\"\t\t\"{}\"\r\n",
                library.to_string_lossy().replace('\\', "\\\\")
            ));
            text.push_str("\t}\r\n");
        }
        text.push_str("}\r\n");
        std::fs::write(self.root.join("steamapps/libraryfolders.vdf"), text).expect("vdf");
    }

    fn install(&self, library: &Path, app_id: &str, install_directory: &str) -> PathBuf {
        let apps = library.join("steamapps");
        let text = format!(
            "\"AppState\"\r\n{{\r\n\t\"appid\"\t\t\"{app_id}\"\r\n\t\"name\"\t\t\"Fixture\"\r\n\t\"installdir\"\t\t\"{install_directory}\"\r\n}}\r\n"
        );
        std::fs::write(apps.join(format!("appmanifest_{app_id}.acf")), text).expect("清单");
        let game = apps.join("common").join(install_directory);
        std::fs::create_dir_all(&game).expect("游戏目录");
        game
    }
}

fn lower(value: &Path) -> String {
    value.to_string_lossy().to_lowercase()
}

#[test]
fn libraries_include_main_and_registered_absolute_paths_only() {
    let steam = Steam::new("libraries");
    let extra = steam.library("Lib-B");
    // 相对路径必须被忽略（与 PS 的 IsPathRooted 一致）
    let vdf = "\"libraryfolders\"\r\n{\r\n\t\"0\"\r\n\t{\r\n\t\t\"path\"\t\t\"relative\\\\Lib\"\r\n\t}\r\n\t\"1\"\r\n\t{\r\n";
    let text = format!(
        "{vdf}\t\t\"path\"\t\t\"{}\"\r\n\t}}\r\n}}\r\n",
        extra.to_string_lossy().replace('\\', "\\\\")
    );
    std::fs::write(steam.root.join("steamapps/libraryfolders.vdf"), text).expect("vdf");

    let directories = library_directories(&steam.executable).expect("库列表");
    assert_eq!(directories.len(), 2, "{directories:?}");
    assert!(
        directories
            .iter()
            .any(|path| lower(path) == lower(&extra.join("steamapps")))
    );
    assert!(
        directories
            .iter()
            .any(|path| lower(path) == lower(&steam.root.join("steamapps")))
    );
}

#[test]
fn find_game_directory_returns_the_single_install() {
    let steam = Steam::new("find");
    let game = steam.install(&steam.root.clone(), APP_ID, "BlackMythWukong");
    let found = find_game_directory(&steam.executable, APP_ID).expect("查找");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(lower(&found[0]), lower(&game));
}

#[test]
fn rooted_installdir_is_ignored() {
    let steam = Steam::new("rooted");
    let apps = steam.root.join("steamapps");
    std::fs::create_dir_all(apps.join("common")).expect("common");
    let text = format!(
        "\"AppState\"\r\n{{\r\n\t\"appid\"\t\t\"{APP_ID}\"\r\n\t\"installdir\"\t\t\"D:\\\\Elsewhere\\\\Game\"\r\n}}\r\n"
    );
    std::fs::write(apps.join(format!("appmanifest_{APP_ID}.acf")), text).expect("清单");
    let found = find_game_directory(&steam.executable, APP_ID).expect("查找");
    assert!(found.is_empty(), "绝对 installdir 必须被忽略：{found:?}");
}

#[test]
fn same_app_in_two_libraries_returns_two_candidates() {
    let steam = Steam::new("duplicate");
    let first = steam.library("Lib-A");
    let second = steam.library("Lib-B");
    steam.library_folders(&[&first, &second]);
    steam.install(&first, APP_ID, "BlackMythWukong");
    steam.install(&second, APP_ID, "BlackMythWukong");

    let found = find_game_directory(&steam.executable, APP_ID).expect("查找");
    assert_eq!(
        found.len(),
        2,
        "两个库都有同一 AppID 时不能只挑一个：{found:?}"
    );
}

#[test]
fn resolve_app_id_uses_the_games_own_library_even_if_unregistered() {
    let steam = Steam::new("resolve");
    let library = steam.library("Lib-Unregistered");
    let game = steam.install(&library, APP_ID, "BlackMythWukong");
    // 故意不写 libraryfolders.vdf：游戏自己的 steamapps 仍是权威来源。

    let context = PathContext::current().expect("上下文");
    let resolved = resolve_app_id(&context, &game, &steam.executable).expect("识别");
    assert_eq!(resolved.app_id, APP_ID);
    assert!(
        resolved
            .manifest
            .ends_with(format!("appmanifest_{APP_ID}.acf"))
    );
}

#[test]
fn manifest_name_mismatch_is_reported_as_invalid() {
    let steam = Steam::new("mismatch");
    let apps = steam.root.join("steamapps");
    std::fs::create_dir_all(apps.join("common/Game")).expect("游戏目录");
    // 文件名是 999，内容却写 123
    let text =
        "\"AppState\"\r\n{\r\n\t\"appid\"\t\t\"123\"\r\n\t\"installdir\"\t\t\"Game\"\r\n}\r\n";
    std::fs::write(apps.join("appmanifest_999.acf"), text).expect("清单");

    let context = PathContext::current().expect("上下文");
    let error = resolve_app_id(&context, &apps.join("common/Game"), &steam.executable)
        .expect_err("应报清单无效");
    assert!(error.to_string().contains("清单无效"), "实际：{error}");
}

#[test]
fn ambiguous_install_directories_are_reported_with_candidates() {
    let steam = Steam::new("ambiguous");
    let apps = steam.root.join("steamapps");
    let game = apps.join("common/Game");
    std::fs::create_dir_all(&game).expect("游戏目录");
    for app_id in ["111", "222"] {
        let text = format!(
            "\"AppState\"\r\n{{\r\n\t\"appid\"\t\t\"{app_id}\"\r\n\t\"installdir\"\t\t\"Game\"\r\n}}\r\n"
        );
        std::fs::write(apps.join(format!("appmanifest_{app_id}.acf")), text).expect("清单");
    }

    let context = PathContext::current().expect("上下文");
    let error = resolve_app_id(&context, &game, &steam.executable).expect_err("应报歧义");
    let message = error.to_string();
    assert!(message.contains("无法唯一识别"), "实际：{message}");
    assert!(
        message.contains("111") && message.contains("222"),
        "应列出候选：{message}"
    );
}

#[test]
fn launch_arguments_use_the_applaunch_flag() {
    assert_eq!(
        launch_arguments(APP_ID).expect("参数"),
        vec!["-applaunch".to_owned(), APP_ID.to_owned()]
    );
    assert!(launch_arguments("0").is_err());
}
