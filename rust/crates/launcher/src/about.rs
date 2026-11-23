//! 关于与日志入口（M4-06）。
//!
//! 界面只展示模型给出的键值，不自己做路径推导，因此这里可以在无窗口环境下测。
//!
//! - 程序版本：来自 `CARGO_PKG_VERSION`
//! - 组件基线：`launcher_core::COMPONENT_BASELINE`（与 `启动脚本` 当前 0.4.2 资源布局一致）
//! - 路径：日志目录、部署记录、偏好文件
//!
//! 通知使用**窗口内**提示（底部状态栏），不弹系统通知，因此不会抢焦点。

use std::path::{Path, PathBuf};

/// 关于对话框的内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AboutInfo {
    pub program_version: String,
    pub component_baseline: String,
    pub logs: PathBuf,
    pub state: PathBuf,
    pub preferences: Option<PathBuf>,
}

impl AboutInfo {
    /// 由启动器根目录与偏好路径构造（不做 IO）。
    pub fn collect(root: &Path, preferences: Option<PathBuf>) -> Self {
        let crate_version = env!("CARGO_PKG_VERSION");
        Self {
            program_version: crate_version.to_owned(),
            component_baseline: launcher_core::COMPONENT_BASELINE.to_owned(),
            logs: logs_directory(root),
            state: root.join(".state"),
            preferences,
        }
    }

    /// 展示用键值对（顺序固定）。
    pub fn lines(&self) -> Vec<(&'static str, String)> {
        let mut lines = vec![
            ("程序版本", self.program_version.clone()),
            ("组件基线", self.component_baseline.clone()),
            ("日志目录", self.logs.display().to_string()),
            ("部署记录", self.state.display().to_string()),
        ];
        if let Some(preferences) = &self.preferences {
            lines.push(("本机偏好", preferences.display().to_string()));
        }
        lines
    }
}

/// 日志目录：`<启动器根>\logs`（与现有 PowerShell 实现同名）。
pub fn logs_directory(root: &Path) -> PathBuf {
    root.join("logs")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_lists_version_baseline_and_paths() {
        let root = Path::new(r"D:\Games\dlssg_for_sm86\rust\target\release");
        let info = AboutInfo::collect(
            root,
            Some(PathBuf::from(r"C:\Users\x\AppData\Local\p.json")),
        );
        let lines = info.lines();
        assert_eq!(lines[0].0, "程序版本");
        assert_eq!(lines[0].1, env!("CARGO_PKG_VERSION"));
        assert_eq!(lines[1].0, "组件基线");
        assert_eq!(lines[1].1, launcher_core::COMPONENT_BASELINE);
        assert_eq!(info.logs, root.join("logs"));
        assert!(lines.iter().any(|(key, value)| *key == "日志目录"
            && value == &info.logs.display().to_string()));
        assert!(lines.iter().any(|(key, _)| *key == "本机偏好"));
    }

    #[test]
    fn missing_preferences_just_omits_the_row() {
        let info = AboutInfo::collect(Path::new(r"D:\root"), None);
        assert!(!info.lines().iter().any(|(key, _)| *key == "本机偏好"));
    }

    #[test]
    fn logs_directory_matches_the_powershell_name() {
        assert_eq!(
            logs_directory(Path::new(r"D:\launcher")),
            PathBuf::from(r"D:\launcher\logs")
        );
    }
}
