//! 本机偏好：已添加的游戏与它们的设置（M4-02）。
//!
//! 与现有 PowerShell 实现的区别（PARITY D-3）：目录名从 `DlssgSteamLauncher` 换成
//! `DlssgLauncher`，旧目录只读兼容；文件结构也简化成新程序自己的 schema。
//!
//! - 路径：`prefs::preferences_path(root)` = `%LOCALAPPDATA%\DlssgLauncher\<16hex>\preferences.json`
//! - 原子写：同目录临时文件 + rename，失败保留旧文件
//! - 缺失文件 = 空偏好（首次启动不是错误）
//! - 未知字段忽略、字段缺失用默认值：旧版本文件不能让新程序起不来

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// 当前 schema 版本。
pub const SCHEMA_VERSION: u32 = 1;

/// 一个已添加的游戏。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameProfile {
    /// 稳定标识：`steam-<AppID>`、`direct-<32 位十六进制>`、`wuthering`、`nte`。
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub directory: PathBuf,
    /// 启动方式：`Steam` / `Epic` / `Direct`。
    #[serde(default = "default_launch_mode")]
    pub launch_mode: String,
    /// 引擎宿主声明：`none` / `UE4` / `UE5`。
    #[serde(default = "default_engine_host")]
    pub engine_host: String,
    /// 画质档位：`Tier1`..`Tier5`。
    #[serde(default = "default_tier")]
    pub quality_tier: String,
    /// 代理编号 1..6。
    #[serde(default = "default_proxy")]
    pub proxy_index: u8,
    /// Steam AppID（`launch_mode=Steam` 时用）。
    #[serde(default)]
    pub app_id: Option<String>,
    /// Epic 的 `AppName`（`launch_mode=Epic` 时用；来自 `LauncherInstalled.dat`）。
    #[serde(default)]
    pub epic_app_name: Option<String>,
    /// 启动参数（Steam 用 `-applaunch <AppID>`；Direct 通常为空）。
    #[serde(default)]
    pub launch_arguments: Option<Vec<String>>,
    /// 官方启动器 EXE（相对游戏目录或绝对路径）；为空时直接启动渲染 EXE。
    #[serde(default)]
    pub launch_executable: Option<String>,
    /// 官方启动器的进程名（默认取 `launch_executable` 的文件名）。
    #[serde(default)]
    pub launcher_process_name: Option<String>,
    /// 明文配置源（相对启动器根或绝对路径）；为空时用 `<根>\配置\<id>\Engine.ini`。
    #[serde(default)]
    pub config_source: Option<String>,
    /// 游戏侧配置目录（绝对路径，最后一级是平台目录）；为空表示不做 Engine.ini 部署。
    #[serde(default)]
    pub config_dir: Option<String>,
    /// 是否部署 `Engine.ini`（默认开；有些游戏不需要改配置）。
    #[serde(default = "default_true")]
    pub deploy_engine_ini: bool,
    /// 会话策略：`first-exit` / `wait-all` / `hot-restart:<秒>`；为空时按 id 推断。
    #[serde(default)]
    pub session_policy: Option<String>,
    /// 渲染 EXE 相对游戏目录的路径（`Binaries\Win64\Game-Win64-Shipping.exe`）。
    #[serde(default)]
    pub render_executable: Option<String>,
    /// 该游戏的 12 键设置；`None` 表示还没在界面里配置过，用组件默认值。
    #[serde(default)]
    pub settings: Option<crate::settings::Settings>,
}

fn default_launch_mode() -> String {
    "Steam".to_owned()
}

fn default_engine_host() -> String {
    "none".to_owned()
}

fn default_tier() -> String {
    "Tier2".to_owned()
}

fn default_proxy() -> u8 {
    1
}

fn default_true() -> bool {
    true
}

impl GameProfile {
    /// 新建一个游戏；缺省值与本机现有偏好一致（Steam / Tier2 / 代理 1）。
    pub fn new(id: &str, name: &str, directory: PathBuf) -> Self {
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            directory,
            launch_mode: default_launch_mode(),
            engine_host: default_engine_host(),
            quality_tier: default_tier(),
            proxy_index: default_proxy(),
            app_id: None,
            epic_app_name: None,
            launch_arguments: None,
            config_source: None,
            launch_executable: None,
            launcher_process_name: None,
            config_dir: None,
            deploy_engine_ini: true,
            session_policy: None,
            render_executable: None,
            settings: None,
        }
    }
}

/// 本机显卡相关的偏好（只记"上次成功切换用的名字"，不含任何备份数据）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuPreference {
    /// 上次成功切换过的目标显示名；第一次用没有值，由界面填默认的 5070。
    #[serde(default)]
    pub target_name: String,
}

/// 偏好文件内容。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preferences {
    /// 运行根（资源与日志所在目录）；为空表示用 exe 所在目录。
    #[serde(default)]
    pub launcher_root: Option<String>,
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub games: Vec<GameProfile>,
    /// 显卡名称切换的本机记忆；没有就是"还没成功切换过"。
    #[serde(default)]
    pub gpu: Option<GpuPreference>,
}

fn default_version() -> u32 {
    SCHEMA_VERSION
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            launcher_root: None,
            games: Vec::new(),
            gpu: None,
        }
    }
}

impl Preferences {
    /// 追加或替换同 ID 的游戏，保持原有顺序。
    pub fn upsert(&mut self, profile: GameProfile) {
        match self.games.iter().position(|item| item.id == profile.id) {
            Some(index) => self.games[index] = profile,
            None => self.games.push(profile),
        }
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.games.len();
        self.games.retain(|item| item.id != id);
        self.games.len() != before
    }

    pub fn find(&self, id: &str) -> Option<&GameProfile> {
        self.games.iter().find(|item| item.id == id)
    }
}

/// 读偏好；文件不存在返回空偏好，格式错误报错（不静默清空用户数据）。
pub fn load(path: &Path) -> Result<Preferences> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Preferences::default());
        }
        Err(error) => return Err(Error::io(path, error)),
    };
    let preferences: Preferences = serde_json::from_str(text.trim()).map_err(|error| {
        Error::invalid(format!("偏好文件无法解析：{}：{error}", path.display()))
    })?;
    Ok(preferences)
}

/// 原子写偏好：先写临时文件再 rename，失败不会破坏旧文件。
pub fn save(path: &Path, preferences: &Preferences) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| Error::io(parent, error))?;
    }
    let text = serde_json::to_string_pretty(preferences)
        .map_err(|error| Error::invalid(format!("偏好无法序列化：{error}")))?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, text.as_bytes()).map_err(|error| Error::io(&temporary, error))?;
    let outcome = std::fs::rename(&temporary, path).map_err(|error| Error::io(path, error));
    if temporary.exists() {
        let _ = std::fs::remove_file(&temporary);
    }
    outcome
}

/// 旧目录（`DlssgSteamLauncher`）里的偏好文件，只读兼容用。
pub fn legacy_path(root: &Path) -> Result<PathBuf> {
    let base = match std::env::var_os("LOCALAPPDATA") {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => {
            return Err(Error::unavailable(
                "环境变量 LOCALAPPDATA 未设置，无法定位旧偏好目录。",
            ));
        }
    };
    Ok(
        crate::prefs::data_directory_in(&base, crate::prefs::LEGACY_DATA_DIRECTORY_NAME, root)?
            .join("preferences.json"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("dlssg-prefs-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("临时目录");
        path.join("preferences.json")
    }

    #[test]
    fn missing_file_means_empty_preferences() {
        let path = scratch("missing");
        let preferences = load(&path).expect("读取");
        assert!(preferences.games.is_empty());
        assert_eq!(preferences.version, SCHEMA_VERSION);
    }

    #[test]
    fn save_then_load_round_trips_and_leaves_no_temp() {
        let path = scratch("round-trip");
        let mut preferences = Preferences::default();
        preferences.upsert(GameProfile::new(
            "steam-2358720",
            "黑神话：悟空",
            PathBuf::from(r"D:\Games\BlackMythWukong"),
        ));
        save(&path, &preferences).expect("保存");
        let loaded = load(&path).expect("读取");
        assert_eq!(loaded, preferences);
        assert!(
            !path.with_extension("json.tmp").exists(),
            "不应残留临时文件"
        );
    }

    #[test]
    fn unknown_fields_are_ignored_and_missing_ones_defaulted() {
        let path = scratch("compat");
        std::fs::write(
            &path,
            r#"{"version":1,"games":[{"id":"nte","name":"异环","directory":"D:\\Games\\NTE","future_field":42}]}"#,
        )
        .expect("写");
        let preferences = load(&path).expect("读取");
        assert_eq!(preferences.games.len(), 1);
        let game = &preferences.games[0];
        assert_eq!(game.launch_mode, "Steam", "缺失字段用默认值");
        assert_eq!(game.engine_host, "none");
        assert_eq!(game.quality_tier, "Tier2");
        assert_eq!(game.proxy_index, 1);
        assert!(game.app_id.is_none(), "缺 app_id 用 None");
        assert!(
            game.render_executable.is_none(),
            "缺 render_executable 用 None"
        );
    }

    #[test]
    fn engine_fields_round_trip() {
        let path = scratch("engine-fields");
        let mut preferences = Preferences::default();
        let mut game = GameProfile::new("wuthering", "鸣潮", PathBuf::from(r"D:\Games\WuWa"));
        game.app_id = Some("4356430".to_owned());
        game.render_executable =
            Some(r"Client\Binaries\Win64\Client-Win64-Shipping.exe".to_owned());
        preferences.upsert(game);
        save(&path, &preferences).expect("保存");
        let loaded = load(&path).expect("读取");
        assert_eq!(loaded.games[0].app_id.as_deref(), Some("4356430"));
        assert_eq!(
            loaded.games[0].render_executable.as_deref(),
            Some(r"Client\Binaries\Win64\Client-Win64-Shipping.exe")
        );
    }

    #[test]
    fn broken_json_is_an_error_not_a_silent_reset() {
        let path = scratch("broken");
        std::fs::write(&path, "{ this is not json").expect("写");
        let error = load(&path).expect_err("应报错");
        assert!(error.to_string().contains("无法解析"), "{error}");
    }

    #[test]
    fn upsert_and_remove_keep_order() {
        let mut preferences = Preferences::default();
        preferences.upsert(GameProfile::new("a", "A", PathBuf::new()));
        preferences.upsert(GameProfile::new("b", "B", PathBuf::new()));
        preferences.upsert(GameProfile::new("a", "A2", PathBuf::new()));
        assert_eq!(preferences.games.len(), 2);
        assert_eq!(preferences.games[0].name, "A2");
        assert!(preferences.remove("a"));
        assert!(!preferences.remove("a"));
        assert_eq!(preferences.games.len(), 1);
        assert_eq!(
            preferences.find("b").map(|item| item.name.as_str()),
            Some("B")
        );
    }

    #[test]
    fn gpu_memory_round_trips_and_old_files_still_load() {
        let path = scratch("gpu");
        let mut preferences = Preferences::default();
        assert!(preferences.gpu.is_none(), "第一次用没有记忆");
        preferences.gpu = Some(GpuPreference {
            target_name: "NVIDIA GeForce RTX 5070".to_owned(),
        });
        save(&path, &preferences).expect("保存");
        let loaded = load(&path).expect("读取");
        assert_eq!(
            loaded.gpu.as_ref().map(|gpu| gpu.target_name.as_str()),
            Some("NVIDIA GeForce RTX 5070")
        );

        // 没有 gpu 字段的旧文件照常读（不能因为加字段就打不开偏好）
        let old = scratch("gpu-legacy");
        std::fs::write(&old, r#"{"version":1,"games":[]}"#).expect("写");
        let loaded = load(&old).expect("旧文件");
        assert!(loaded.gpu.is_none());
    }
}
