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
    /// 检测到该游戏的进程正在运行（渲染 EXE 或官方启动器）。
    ///
    /// 与 `status` 分开：手动启动的游戏也会是 `true`，此时状态可能仍是"未部署"。
    pub running: bool,
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

/// 由**这一行自己**的事实推导状态：目录是否存在、是否有未完成部署、这个游戏是否在跑。
///
/// 注意 `running` 必须是"本行的游戏进程在跑"，不能传全局会话状态：那样一款游戏在跑
/// 会让所有行都显示"运行中"，而真正在跑的那一行反而显示"已部署"（2026-11-23 真机反馈）。
/// 全局"另一款游戏占着会话"只用来禁用按钮，不参与每一行的状态文字。
pub fn derive_status(
    directory_exists: bool,
    has_pending_deployment: bool,
    running: bool,
) -> GameStatus {
    if !directory_exists {
        GameStatus::Missing
    } else if running {
        // 游戏在跑时优先显示"运行中"，即使还有未清理的部署记录。
        GameStatus::SessionActive
    } else if has_pending_deployment {
        GameStatus::NeedsRestore
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
    /// 全局引擎会话是否被占（只有一行能跑）。只用来禁用别的行的「启动」，
    /// **不参与**每一行的状态文字。
    pub session_taken: bool,
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

    /// 按 id 选中（行内按钮用：按钮回调里拿到的只有 id）。
    pub fn select_by_id(&mut self, id: &str) {
        if let Some(index) = self.games.iter().position(|entry| entry.id == id) {
            self.selected = Some(index);
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

    /// 启动按钮是否可用（行内按钮已改用它处判断，这里保留给测试）。
    #[allow(dead_code)]
    pub fn can_launch(&self) -> bool {
        let Some(entry) = self.selected_game() else {
            return false;
        };
        entry.status.can_launch() && !self.session_busy()
    }

    /// 恢复按钮是否可用。
    #[allow(dead_code)]
    pub fn can_restore(&self) -> bool {
        self.selected_game()
            .map(|entry| entry.status.can_restore())
            .unwrap_or(false)
    }

    /// 这一行自己的游戏在跑（按该行的状态判断）。
    pub fn row_running(&self, index: usize) -> bool {
        self.games
            .get(index)
            .map(|entry| entry.running || entry.status == GameStatus::SessionActive)
            .unwrap_or(false)
    }

    /// 是否有任意一行在跑（只用于禁用别的行的「启动」）。
    pub fn session_running(&self) -> bool {
        self.session_taken || self.games.iter().any(|entry| entry.running)
    }

    /// 同一件事的别名，语义上强调"会话被占用"。
    pub fn session_busy(&self) -> bool {
        self.session_running()
    }

    /// 标题栏用的状态文案。
    pub fn headline(&self) -> String {
        if self.is_empty() {
            return "还没有添加游戏".to_owned();
        }
        if let Some(entry) = self.games.iter().find(|entry| entry.running) {
            return format!("{} 运行中", entry.name);
        }
        if self.session_taken {
            return "引擎会话进行中".to_owned();
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
            running: false,
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

        // 另一个游戏在跑 → 全局禁用启动（但它的状态只属于它自己那一行）
        view.upsert(entry("nte", "异环", GameStatus::SessionActive));
        view.games[1].running = true;
        assert!(view.session_running());
        assert!(!view.can_launch());
        assert_eq!(
            view.games[0].status,
            GameStatus::Ready,
            "鸣潮那行不该跟着变"
        );
        assert_eq!(view.headline(), "异环 运行中");

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
    fn derive_status_is_per_game_and_running_wins_over_pending() {
        // 目录缺失优先
        assert_eq!(derive_status(false, true, true), GameStatus::Missing);
        // 本行在跑：即使有未清理部署也显示"运行中"
        assert_eq!(derive_status(true, true, true), GameStatus::SessionActive);
        assert_eq!(derive_status(true, false, true), GameStatus::SessionActive);
        // 有部署但本行没在跑
        assert_eq!(derive_status(true, true, false), GameStatus::NeedsRestore);
        assert_eq!(derive_status(true, false, false), GameStatus::Ready);
        assert_eq!(GameStatus::Missing.text(), "目录缺失");
        assert!(!GameStatus::Missing.can_launch());
        assert!(!GameStatus::Missing.can_restore());
    }

    #[test]
    fn running_a_game_does_not_change_the_other_rows() {
        let mut view = LauncherView::new();
        view.upsert(entry("wuwa", "鸣潮", GameStatus::Ready));
        view.upsert(entry("nte", "异环", GameStatus::SessionActive));
        view.games[1].running = true;
        // 只有异环那行是"在跑"，鸣潮那行保持原样
        assert_eq!(view.games[0].status, GameStatus::Ready);
        assert_eq!(view.games[1].status, GameStatus::SessionActive);
        assert!(view.row_running(1));
        assert!(!view.row_running(0));
        // 但全局会话被占用：别的行不能再启动
        assert!(view.session_busy());
        assert!(view.headline().contains("异环"), "{}", view.headline());

        // 引擎占着会话但没有任何一行在跑时，标题说明这一点，行的状态仍是各自的事实
        let mut idle = LauncherView::new();
        idle.upsert(entry("wuwa", "鸣潮", GameStatus::Ready));
        idle.session_taken = true;
        assert!(idle.session_busy());
        assert_eq!(
            idle.games[0].status,
            GameStatus::Ready,
            "不能把全局会话写进行状态"
        );
        assert_eq!(idle.headline(), "引擎会话进行中");
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
