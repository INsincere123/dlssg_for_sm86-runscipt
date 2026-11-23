//! 列表标题改名：双击标题进入，回车保存，Esc 放弃。
//!
//! 与 [`crate::add_game`] 一样只放决策与校验，`ui.rs` 负责画输入框、收按键。
//! 改名**只动偏好里的 `name`**：id、目录、部署记录都不变，所以改完不影响启动、
//! 恢复或引擎请求（引擎按 id + 目录定位，名字只出现在提示与日志里）。
//!
//! 需要知道的副作用：`engine::default_engine_source` 在 `<根>\配置\<id>\Engine.ini`
//! 不存在时会退到 `<根>\配置\<游戏名>\Engine.ini`——改了名字就等于改了这条兜底路径。
//! 档案里显式写了 `config_source` 的游戏（扫描/内置模板加进来的那些）不受影响。

/// 名字长度上限：名字是行内标题，太长会把同一行的按钮挤出去。
pub const MAX_NAME_CHARS: usize = 60;

/// 一行正在编辑的标题。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameState {
    /// 要改的游戏 id（稳定标识，不随名字变）。
    pub id: String,
    /// 开始编辑时的名字：取消时直接用，不必再读偏好。
    pub original: String,
    /// 输入框里的文字。
    pub text: String,
    /// 还没要过焦点：下一帧请求焦点并全选，用户直接输入就能覆盖旧名。
    pub fresh: bool,
}

impl RenameState {
    pub fn start(id: &str, name: &str) -> Self {
        Self {
            id: id.to_owned(),
            original: name.to_owned(),
            text: name.to_owned(),
            fresh: true,
        }
    }

    /// 提交时用的名字：去首尾空白；空的或太长的给一句人话，不写进偏好。
    pub fn normalized(&self) -> Result<String, String> {
        let trimmed = self.text.trim();
        if trimmed.is_empty() {
            return Err("游戏名称不能为空，已保留原名".to_owned());
        }
        let length = trimmed.chars().count();
        if length > MAX_NAME_CHARS {
            return Err(format!(
                "游戏名称太长（{length} 字，最多 {MAX_NAME_CHARS} 字），已保留原名"
            ));
        }
        Ok(trimmed.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_keeps_the_old_name_and_asks_for_focus() {
        let state = RenameState::start("wuthering", "鸣潮");
        assert_eq!(state.id, "wuthering");
        assert_eq!(state.original, "鸣潮");
        assert_eq!(state.text, "鸣潮", "输入框先放原名，用户可以改几个字");
        assert!(state.fresh, "进入编辑时要有一次请求焦点");
    }

    #[test]
    fn commit_trims_surrounding_whitespace() {
        let mut state = RenameState::start("nte", "异环");
        state.text = "  异环（NTE）  ".to_owned();
        assert_eq!(state.normalized().as_deref(), Ok("异环（NTE）"));
    }

    #[test]
    fn empty_name_is_refused() {
        let mut state = RenameState::start("nte", "异环");
        state.text = "   ".to_owned();
        let error = state.normalized().expect_err("空名字要拒绝");
        assert!(error.contains("不能为空"), "{error}");
    }

    #[test]
    fn overly_long_name_is_refused() {
        let mut state = RenameState::start("nte", "异环");
        state.text = "名".repeat(MAX_NAME_CHARS + 1);
        let error = state.normalized().expect_err("超长要拒绝");
        assert!(error.contains("太长"), "{error}");
    }

    #[test]
    fn the_limit_itself_is_accepted() {
        let mut state = RenameState::start("nte", "异环");
        state.text = "名".repeat(MAX_NAME_CHARS);
        assert_eq!(
            state.normalized().map(|name| name.chars().count()),
            Ok(MAX_NAME_CHARS)
        );
    }

    #[test]
    fn whitespace_only_edits_normalize_to_the_old_name() {
        let mut state = RenameState::start("wuthering", "鸣潮");
        state.text = " 鸣潮\n".to_owned();
        assert_eq!(
            state.normalized().as_deref(),
            Ok("鸣潮"),
            "只改空白就等于原名，提交时不该写盘"
        );
        assert_eq!(state.normalized().as_deref(), Ok(state.original.as_str()));
    }
}
