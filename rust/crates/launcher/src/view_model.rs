//! 界面模型（M4-01/02）：不依赖 egui，可在无窗口环境下测。
//!
//! 界面只做三件事：列出已添加的游戏、显示每个游戏的状态、把用户操作翻译成引擎请求。
//! 所有判断都在这里，`ui.rs` 只负责画。

use std::path::PathBuf;

/// 一个游戏在界面上显示的状态。
// 变体由引擎回调（M4-05 起）与测试构造，界面只消费。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameStatus {
    /// 可以启动（无记录或记录完整）。
    Ready,
    /// 存在未完成的部署，必须先恢复。
    NeedsRestore,
    /// 引擎会话进行中（另一个游戏在跑）。
    SessionActive,
    /// 游戏目录不存在（被移动或删除）。
    Missing,
}

impl GameStatus {
    pub fn text(self) -> &'static str {
        match self {
            Self::Ready => "未部署（可启动）",
            Self::NeedsRestore => "已部署（可恢复）",
            Self::SessionActive => "运行中",
            Self::Missing => "目录缺失",
        }
    }

    /// 是否允许"启动"按钮。
    pub fn can_launch(self) -> bool {
        matches!(self, Self::Ready)
    }

    /// 是否允许"恢复"按钮。
    pub fn can_restore(self) -> bool {
        matches!(self, Self::NeedsRestore | Self::SessionActive)
    }
}

/// 列表里的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameEntry {
    pub id: String,
    pub name: String,
    /// 游戏目录（空字符串表示尚未定位）。
    pub directory: PathBuf,
    pub status: GameStatus,
    /// 引擎宿主声明：`none` / `UE4` / `UE5`。
    pub engine_host: String,
}

impl GameEntry {
    pub fn summary(&self) -> String {
        if self.directory.as_os_str().is_empty() {
            format!("{} · 未定位目录 · {}", self.name, self.status.text())
        } else {
            format!(
                "{} · {} · {}",
                self.name,
                self.directory.display(),
                self.status.text()
            )
        }
    }
}

/// 由事实推导状态：目录是否存在、是否有未完成部署、是否有会话在跑。
pub fn derive_status(
    directory_exists: bool,
    has_pending_deployment: bool,
    session_active: bool,
) -> GameStatus {
    if !directory_exists {
        GameStatus::Missing
    } else if has_pending_deployment {
        GameStatus::NeedsRestore
    } else if session_active {
        GameStatus::SessionActive
    } else {
        GameStatus::Ready
    }
}

/// 主窗口模型。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LauncherView {
    pub games: Vec<GameEntry>,
    pub selected: Option<usize>,
    /// 底部提示（错误或进度），由引擎回调写入。
    pub notice: String,
}

impl LauncherView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.games.is_empty()
    }

    /// 追加或替换同 ID 的游戏（M4-05 添加游戏后调用）。
    #[allow(dead_code)]
    pub fn upsert(&mut self, entry: GameEntry) {
        match self.games.iter().position(|item| item.id == entry.id) {
            Some(index) => self.games[index] = entry,
            None => self.games.push(entry),
        }
    }

    pub fn select(&mut self, index: usize) {
        if index < self.games.len() {
            self.selected = Some(index);
        }
    }

    pub fn selected_game(&self) -> Option<&GameEntry> {
        self.selected.and_then(|index| self.games.get(index))
    }

    /// 启动按钮是否可用：选中游戏必须"可启动"，且没有正在进行的会话。
    pub fn can_launch(&self) -> bool {
        let Some(entry) = self.selected_game() else {
            return false;
        };
        entry.status.can_launch() && !self.session_running()
    }

    /// 恢复按钮是否可用。
    pub fn can_restore(&self) -> bool {
        self.selected_game()
            .map(|entry| entry.status.can_restore())
            .unwrap_or(false)
    }

    pub fn session_running(&self) -> bool {
        self.games
            .iter()
            .any(|entry| entry.status == GameStatus::SessionActive)
    }

    /// 标题栏用的状态文案。
    pub fn headline(&self) -> String {
        if self.is_empty() {
            return "还没有添加游戏".to_owned();
        }
        if self.session_running() {
            return "游戏运行中".to_owned();
        }
        format!("已添加 {} 个游戏", self.games.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, name: &str, status: GameStatus) -> GameEntry {
        GameEntry {
            id: id.to_owned(),
            name: name.to_owned(),
            directory: PathBuf::from(format!(r"D:\Games\{name}")),
            status,
            engine_host: "UE5".to_owned(),
        }
    }

    #[test]
    fn empty_view_cannot_launch_or_restore() {
        let view = LauncherView::new();
        assert!(view.is_empty());
        assert!(!view.can_launch());
        assert!(!view.can_restore());
        assert_eq!(view.headline(), "还没有添加游戏");
    }

    #[test]
    fn upsert_replaces_same_id_and_appends_others() {
        let mut view = LauncherView::new();
        view.upsert(entry("wuwa", "鸣潮", GameStatus::Ready));
        view.upsert(entry("nte", "异环", GameStatus::Ready));
        assert_eq!(view.games.len(), 2);
        view.upsert(entry("wuwa", "鸣潮", GameStatus::NeedsRestore));
        assert_eq!(view.games.len(), 2, "同 ID 应替换而不是追加");
        assert_eq!(view.games[0].status, GameStatus::NeedsRestore);
    }

    #[test]
    fn launch_requires_a_ready_selection_and_no_running_session() {
        let mut view = LauncherView::new();
        view.upsert(entry("wuwa", "鸣潮", GameStatus::Ready));
        assert!(!view.can_launch(), "未选中时不可启动");
        view.select(0);
        assert!(view.can_launch());

        // 另一个游戏在跑 → 全局禁用启动
        view.upsert(entry("nte", "异环", GameStatus::SessionActive));
        assert!(view.session_running());
        assert!(!view.can_launch());
        assert_eq!(view.headline(), "游戏运行中");

        // 需要恢复的游戏不能启动，但可以恢复
        let mut needs = LauncherView::new();
        needs.upsert(entry("wuwa", "鸣潮", GameStatus::NeedsRestore));
        needs.select(0);
        assert!(!needs.can_launch());
        assert!(needs.can_restore());
    }

    #[test]
    fn status_texts_and_summaries_are_stable() {
        assert_eq!(GameStatus::Ready.text(), "未部署（可启动）");
        assert_eq!(GameStatus::NeedsRestore.text(), "已部署（可恢复）");
        assert_eq!(GameStatus::SessionActive.text(), "运行中");
        assert!(GameStatus::Ready.can_launch());
        assert!(!GameStatus::SessionActive.can_launch());
        assert!(GameStatus::NeedsRestore.can_restore());

        let located = entry("wuwa", "鸣潮", GameStatus::Ready);
        assert!(located.summary().contains("D:\\Games\\鸣潮"));
        let mut missing = located.clone();
        missing.directory = PathBuf::new();
        assert!(missing.summary().contains("未定位目录"));
    }

    #[test]
    fn derive_status_orders_missing_before_pending_and_session() {
        assert_eq!(derive_status(false, true, true), GameStatus::Missing);
        assert_eq!(derive_status(true, true, true), GameStatus::NeedsRestore);
        assert_eq!(derive_status(true, false, true), GameStatus::SessionActive);
        assert_eq!(derive_status(true, false, false), GameStatus::Ready);
        assert_eq!(GameStatus::Missing.text(), "目录缺失");
        assert!(!GameStatus::Missing.can_launch());
        assert!(!GameStatus::Missing.can_restore());
    }

    #[test]
    fn select_ignores_out_of_range() {
        let mut view = LauncherView::new();
        view.upsert(entry("wuwa", "鸣潮", GameStatus::Ready));
        view.select(5);
        assert_eq!(view.selected, None);
        view.select(0);
        assert_eq!(
            view.selected_game().map(|item| item.id.as_str()),
            Some("wuwa")
        );
    }
}
