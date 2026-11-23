//! 添加游戏（M4-05）：选目录 → 扫描候选 → 确认渲染程序 → 声明引擎宿主。
//!
//! 模型与界面分离：这里只做决策，`ui.rs` 负责画和收集输入。
//! 扫描本身在 `discovery` 里（12000 目录 / 12 层上限、可取消）；
//! 本模块只消费结果，因此可在无窗口环境下用构造好的 outcome 测试。

use std::path::{Path, PathBuf};

use launcher_core::discovery::{DiscoveryOutcome, RendererCandidate, trusted_renderer};
use launcher_core::preferences::GameProfile;

/// 引擎宿主三选一（PARITY 决策 3.6：声明而非自动识别）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HostChoice {
    #[default]
    None,
    Ue4,
    Ue5,
}

impl HostChoice {
    pub fn as_text(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Ue4 => "UE4",
            Self::Ue5 => "UE5",
        }
    }

    /// 由偏好文件里的文本还原（M4-05 之外由设置页复用）。
    #[allow(dead_code)]
    pub fn from_text(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "ue4" => Self::Ue4,
            "ue5" => Self::Ue5,
            _ => Self::None,
        }
    }

    pub fn all() -> [Self; 3] {
        [Self::None, Self::Ue4, Self::Ue5]
    }
}

/// 添加游戏对话框的状态。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AddGameState {
    /// 用户填写的游戏目录。
    pub directory: String,
    pub host: HostChoice,
    pub candidates: Vec<RendererCandidate>,
    /// 用户选中的候选；扫描结束后按内部置信度默认预选一个。
    pub selected: Option<usize>,
    /// 内部判定：置信度是否够高（唯一且分数 ≥60）；分数本身不显示给用户。
    pub trusted: bool,
    /// 扫描过程中或结果提示。
    pub message: String,
}

impl AddGameState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 预先填好目录（扫描结果里「需要确认」那一项用）。
    pub fn prefilled(directory: &Path) -> Self {
        Self {
            directory: directory.display().to_string(),
            message: if directory.is_dir() {
                "目录已填好，点「扫描」找渲染程序".to_owned()
            } else {
                "目录不存在".to_owned()
            },
            ..Self::default()
        }
    }

    /// 目录是否可用（非空、存在、是文件夹）。
    pub fn directory_ok(&self) -> bool {
        let path = Path::new(self.directory.trim());
        !self.directory.trim().is_empty() && path.is_dir()
    }

    /// 消费一次扫描结果：填候选、按内部置信度预选、写提示。
    pub fn apply_scan(&mut self, outcome: DiscoveryOutcome) {
        self.candidates = outcome.candidates;
        self.trusted = trusted_renderer(&self.candidates).is_some();
        self.selected = if self.trusted { Some(0) } else { None };
        // 置信度只影响"要不要替用户预选"，不进提示文案。
        self.message = if self.candidates.is_empty() {
            "没有找到可用的 x64 程序；请确认目录，或稍后手动指定。".to_owned()
        } else if self.trusted {
            format!(
                "找到 {} 个候选，已自动预选第一个；不对就手动选。",
                self.candidates.len()
            )
        } else {
            format!("找到 {} 个候选，请手动选择一个。", self.candidates.len())
        };
        if !outcome.issues.is_empty() {
            self.message
                .push_str(&format!("（{}）", outcome.issues.join("；")));
        }
    }

    pub fn select(&mut self, index: usize) {
        if index < self.candidates.len() {
            self.selected = Some(index);
        }
    }

    pub fn selected_candidate(&self) -> Option<&RendererCandidate> {
        self.selected.and_then(|index| self.candidates.get(index))
    }

    /// 确认添加：需要目录可用且已选中候选；`None` 表示还不能确定。
    pub fn confirm(&self) -> Option<GameProfile> {
        if !self.directory_ok() {
            return None;
        }
        let candidate = self.selected_candidate()?;
        let directory = PathBuf::from(self.directory.trim());
        let name = directory
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| directory.display().to_string());
        // 手动添加一律走 Direct：渲染程序已由扫描确定，不需要 Steam 清单。
        let id = launcher_core::direct::new_direct_id(&candidate.path.to_string_lossy());
        // 选中的渲染程序必须写进偏好，否则引擎无法定位（曾漏掉导致计划阶段失败）。
        let relative = candidate
            .path
            .strip_prefix(&directory)
            .map(|value| value.to_string_lossy().replace('/', "\\"))
            .ok()
            .unwrap_or_else(|| candidate.relative.clone());
        let mut profile = GameProfile::new(&id, &name, directory);
        profile.launch_mode = "Direct".to_owned();
        profile.engine_host = self.host.as_text().to_owned();
        profile.render_executable = Some(relative);
        Some(profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use launcher_core::discovery::DiscoveryLimits;

    fn candidate(relative: &str, score: i32) -> RendererCandidate {
        RendererCandidate {
            path: PathBuf::from(format!(r"D:\Games\Fixture\{relative}")),
            relative: relative.to_owned(),
            score,
            reasons: vec!["测试".to_owned()],
        }
    }

    fn outcome(items: Vec<RendererCandidate>) -> DiscoveryOutcome {
        DiscoveryOutcome {
            candidates: items,
            ..DiscoveryOutcome::default()
        }
    }

    fn state_with_dir(name: &str) -> (AddGameState, PathBuf) {
        let directory =
            std::env::temp_dir().join(format!("dlssg-addgame-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("临时目录");
        let mut state = AddGameState::new();
        state.directory = directory.to_string_lossy().into_owned();
        (state, directory)
    }

    #[test]
    fn only_a_trusted_candidate_is_pre_selected() {
        let (mut state, _directory) = state_with_dir("trusted");
        state.apply_scan(outcome(vec![
            candidate("Binaries/Win64/Game-Win64-Shipping.exe", 140),
            candidate("Other.exe", 0),
        ]));
        assert!(state.trusted);
        assert_eq!(state.selected, Some(0));
        assert!(state.message.contains("自动预选"));

        let (mut tied, _directory2) = state_with_dir("tied");
        tied.apply_scan(outcome(vec![
            candidate("A.exe", 60),
            candidate("B.exe", 60),
        ]));
        assert!(!tied.trusted, "同分不可信");
        assert_eq!(tied.selected, None);
        assert!(tied.message.contains("手动选择"));
    }

    #[test]
    fn empty_scan_explains_itself() {
        let (mut state, _directory) = state_with_dir("empty");
        state.apply_scan(outcome(Vec::new()));
        assert!(state.candidates.is_empty());
        assert!(state.message.contains("没有找到"));
        assert!(state.confirm().is_none());
    }

    #[test]
    fn confirm_builds_a_direct_profile_with_the_declared_host() {
        let (mut state, directory) = state_with_dir("confirm");
        state.apply_scan(outcome(vec![candidate(
            "Binaries/Win64/Game-Win64-Shipping.exe",
            140,
        )]));
        state.host = HostChoice::Ue5;
        let profile = state.confirm().expect("应能确认");
        assert_eq!(profile.launch_mode, "Direct");
        assert_eq!(profile.engine_host, "UE5");
        assert_eq!(profile.directory, directory);
        assert!(
            launcher_core::direct::validate_direct_id(&profile.id).is_ok(),
            "{}",
            profile.id
        );
        assert_eq!(
            profile.name,
            directory.file_name().unwrap().to_string_lossy()
        );
    }

    #[test]
    fn confirm_requires_directory_and_selection() {
        let mut state = AddGameState::new();
        state.apply_scan(outcome(vec![candidate("Game.exe", 60)]));
        assert!(state.confirm().is_none(), "目录为空不能确认");

        let (mut good, _directory) = state_with_dir("no-selection");
        good.apply_scan(outcome(vec![
            candidate("A.exe", 60),
            candidate("B.exe", 60),
        ]));
        assert_eq!(good.selected, None);
        assert!(good.confirm().is_none(), "未选候选不能确认");

        good.select(1);
        assert!(good.confirm().is_some(), "手动选中后可以确认");
    }

    #[test]
    fn host_text_round_trips() {
        for host in HostChoice::all() {
            assert_eq!(HostChoice::from_text(host.as_text()), host);
        }
        assert_eq!(HostChoice::from_text("ue4"), HostChoice::Ue4);
        assert_eq!(HostChoice::from_text("bogus"), HostChoice::None);
    }

    #[test]
    fn scan_limits_are_the_default_budget() {
        let limits = DiscoveryLimits::default();
        assert_eq!(limits.max_directories, 12_000);
        assert_eq!(limits.max_depth, 12);
    }
}
