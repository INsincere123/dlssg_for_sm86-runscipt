//! 界面 → 引擎的交接（M4-06）：拼参数、提权拉起工作进程，然后界面退出。
//!
//! 界面本身是 `asInvoker`，真正干活的是提权后的隐藏子进程；
//! 这样界面进程的内存可以在交接后立刻释放。

use std::path::Path;

/// 交接命令：`run` 启动游戏，`restore` 恢复原状，`stop` 结束进程后恢复。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Run,
    Restore,
    Stop,
}

impl Command {
    pub fn as_str(self) -> &'static str {
        match self {
            Command::Run => "run",
            Command::Restore => "restore",
            Command::Stop => "stop",
        }
    }

    /// 报告文件名（界面按它轮询结果）。
    pub fn report_name(self) -> &'static str {
        match self {
            Command::Run => "ui-run.json",
            Command::Restore => "ui-restore.json",
            Command::Stop => "ui-stop.json",
        }
    }
}

/// 拼出交给子进程的参数串（引号由这里统一加，调用方不要重复加）。
pub fn engine_arguments(command: Command, game: &str, root: &Path, report: &Path) -> String {
    format!(
        "--engine {} --game \"{}\" --root \"{}\" --report \"{}\"",
        command.as_str(),
        game,
        root.display(),
        report.display()
    )
}

/// 提权拉起工作进程；成功后调用方应当退出界面。
pub fn hand_off(
    command: Command,
    game: &str,
    root: &Path,
    report: &Path,
) -> launcher_core::Result<()> {
    let executable = std::env::current_exe()
        .map_err(|error| launcher_core::Error::invalid(format!("取自身路径失败：{error}")))?;
    let arguments = engine_arguments(command, game, root, report);
    launcher_win::elevate::spawn_elevated(&executable, &arguments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_arguments_carry_game_root_and_report() {
        let text = engine_arguments(
            Command::Run,
            "direct-896a",
            Path::new(r"D:\run root"),
            Path::new(r"D:\run root\logs\ui-run.json"),
        );
        assert_eq!(
            text,
            r#"--engine run --game "direct-896a" --root "D:\run root" --report "D:\run root\logs\ui-run.json""#
        );
        assert!(!text.contains("--dry-run"), "界面启动不应是演练：{text}");
    }

    #[test]
    fn restore_arguments_use_the_restore_subcommand() {
        let text = engine_arguments(
            Command::Restore,
            "nte",
            Path::new(r"C:\root"),
            Path::new(r"C:\root\ui-restore.json"),
        );
        assert!(text.starts_with("--engine restore "), "{text}");
        assert!(text.contains(r#"--game "nte""#), "{text}");
    }
}
