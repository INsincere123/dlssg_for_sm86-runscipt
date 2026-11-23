//! `launcher`：单 EXE 入口（M2-02）。
//!
//! 现在是**引擎壳**：`--engine check` 做环境自检并写报告；`run` / `restore` 先返回
//! "未实现"，等 M2-03（提权）、M2-04（IPC）、M2-05（单会话）接上。
//!
//! 关键约束（PARITY 2.1 / 2.2）：整个二进制是 `windows` 子系统，没有控制台窗口；
//! 引擎必须在命令行下也不弹窗，因此**永远不向 stdout 写产品输出**，
//! 结果一律落到 `--report` 指定的 JSON 文件里。

#![windows_subsystem = "windows"]

use std::path::PathBuf;
use std::process::ExitCode;

const EXIT_OK: u8 = 0;
const EXIT_USAGE: u8 = 2;
const EXIT_NOT_IMPLEMENTED: u8 = 3;
const EXIT_CHILD_FAILED: u8 = 4;
const EXIT_ELEVATION_CANCELLED: u8 = 5;
const EXIT_SESSION_BUSY: u8 = 6;

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(failure) => {
            // 没有控制台：消息写到 stderr（通常被丢弃），主要靠退出码表达。
            let (code, message) = match &failure {
                Failure::Usage(message) => (EXIT_USAGE, message),
                Failure::NotImplemented(message) => (EXIT_NOT_IMPLEMENTED, message),
            };
            eprintln!("{message}");
            // 没有控制台时，失败原因只能落盘：把 --report 里的文件写成错误报告。
            if let Some(path) = report_path_from_env() {
                write_report(
                    Some(&path),
                    &format!("{{\n  \"error\": \"{}\"\n}}\n", message.replace('"', "'")),
                );
            }
            ExitCode::from(code)
        }
    }
}

enum Failure {
    Usage(String),
    NotImplemented(String),
}

struct Arguments {
    engine: Option<String>,
    report: Option<PathBuf>,
    game: Option<String>,
    elevate: bool,
    hold_ms: u64,
    launcher_name: Option<String>,
}

fn parse(arguments: &[String]) -> Result<Arguments, Failure> {
    let mut parsed = Arguments {
        engine: None,
        report: None,
        game: None,
        elevate: false,
        hold_ms: 0,
        launcher_name: None,
    };
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--engine" => {
                parsed.engine = Some(
                    arguments
                        .get(index + 1)
                        .ok_or_else(|| Failure::Usage("--engine 缺少值".to_owned()))?
                        .clone(),
                );
                index += 2;
            }
            "--report" => {
                parsed.report =
                    Some(PathBuf::from(arguments.get(index + 1).ok_or_else(
                        || Failure::Usage("--report 缺少值".to_owned()),
                    )?));
                index += 2;
            }
            "--game" => {
                parsed.game = Some(
                    arguments
                        .get(index + 1)
                        .ok_or_else(|| Failure::Usage("--game 缺少值".to_owned()))?
                        .clone(),
                );
                index += 2;
            }
            "--launcher-name" => {
                parsed.launcher_name = Some(
                    arguments
                        .get(index + 1)
                        .ok_or_else(|| Failure::Usage("--launcher-name 缺少值".to_owned()))?
                        .clone(),
                );
                index += 2;
            }
            "--hold-ms" => {
                parsed.hold_ms = arguments
                    .get(index + 1)
                    .ok_or_else(|| Failure::Usage("--hold-ms 缺少值".to_owned()))?
                    .parse()
                    .map_err(|_| Failure::Usage("--hold-ms 需要非负整数".to_owned()))?;
                index += 2;
            }
            "--elevate" => {
                parsed.elevate = true;
                index += 1;
            }
            "--version" => {
                parsed.engine = Some("version".to_owned());
                index += 1;
            }
            other => return Err(Failure::Usage(format!("未知参数：{other}"))),
        }
    }
    Ok(parsed)
}

fn run() -> Result<u8, Failure> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let parsed = parse(&arguments)?;
    let Some(command) = parsed.engine.as_deref() else {
        return Err(Failure::Usage(
            "用法：--engine check|run|restore [--game <id>] [--report <json>]".to_owned(),
        ));
    };

    match command {
        "version" => {
            write_report(
                parsed.report.as_deref(),
                &format!("{{\"version\":\"{}\"}}", env!("CARGO_PKG_VERSION")),
            );
            Ok(EXIT_OK)
        }
        "check" if parsed.elevate => {
            // 自我提权：把同样的 check 交给一个提权子进程，父进程只记录结果。
            let child_report = parsed
                .report
                .as_ref()
                .map(|path| path.with_file_name("elevated-child.json"))
                .ok_or_else(|| Failure::Usage("--elevate 需要 --report".to_owned()))?;
            let exe = std::env::current_exe()
                .map_err(|error| Failure::Usage(format!("取自身路径失败：{error}")))?;
            let arguments = format!("--engine check --report \"{}\"", child_report.display());
            let outcome = launcher_win::elevate::run_elevated(
                &exe,
                &arguments,
                std::time::Duration::from_secs(120),
            )
            .map_err(|error| Failure::Usage(format!("提权启动失败：{error}")))?;
            let report = match outcome {
                launcher_win::elevate::ElevationOutcome::Finished { code } => format!(
                    "{{\n  \"command\": \"check\",\n  \"elevation\": \"finished\",\n  \"child_exit\": {code},\n  \"child_report\": \"{child}\"\n}}\n",
                    child = child_report.display()
                ),
                launcher_win::elevate::ElevationOutcome::Cancelled => {
                    "{\n  \"command\": \"check\",\n  \"elevation\": \"cancelled\"\n}\n".to_owned()
                }
            };
            write_report(parsed.report.as_deref(), &report);
            match outcome {
                launcher_win::elevate::ElevationOutcome::Finished { code: 0 } => Ok(EXIT_OK),
                launcher_win::elevate::ElevationOutcome::Finished { .. } => Ok(EXIT_CHILD_FAILED),
                launcher_win::elevate::ElevationOutcome::Cancelled => Ok(EXIT_ELEVATION_CANCELLED),
            }
        }
        "check" => {
            let report = format!(
                concat!(
                    "{{\n",
                    "  \"command\": \"check\",\n",
                    "  \"version\": \"{version}\",\n",
                    "  \"pid\": {pid},\n",
                    "  \"console_window\": {console},\n",
                    "  \"elevated\": {elevated},\n",
                    "  \"cpu\": \"unknown\"\n",
                    "}}\n"
                ),
                version = env!("CARGO_PKG_VERSION"),
                pid = std::process::id(),
                console = launcher_win::console_window(),
                elevated = launcher_win::is_elevated(),
            );
            write_report(parsed.report.as_deref(), &report);
            Ok(EXIT_OK)
        }
        "run" => {
            // 单会话 + 状态发布的最小闭环；游戏会话本体在 M2-06 之后接入。
            let game = parsed
                .game
                .clone()
                .ok_or_else(|| Failure::Usage("--engine run 需要 --game".to_owned()))?;
            let guard = match launcher_win::acquire_single_session(std::time::Duration::ZERO) {
                Ok(Some(guard)) => guard,
                Ok(None) => {
                    write_report(
                        parsed.report.as_deref(),
                        "{\n  \"code\": \"session-busy\",\n  \"message\": \"另一个游戏会话正在运行\"\n}\n",
                    );
                    return Ok(EXIT_SESSION_BUSY);
                }
                Err(error) => return Err(Failure::Usage(format!("单会话互斥不可用：{error}"))),
            };
            // 复用已运行的官方启动器（M2-06）：命中就不再拉起。
            let mut launcher_running = false;
            if let Some(name) = parsed.launcher_name.as_deref() {
                let spec = launcher_core::session::SessionSpec::new(
                    launcher_core::session::SessionPolicy::FirstExit,
                )
                .with_game(&game)
                .with_launcher(name);
                launcher_running = launcher_core::session::launcher_already_running(
                    &launcher_win::WinProcessProbe,
                    &spec,
                )
                .unwrap_or(false);
            }
            let status_path = parsed
                .report
                .as_ref()
                .map(|path| path.with_file_name("status.json"));
            if let Some(path) = status_path.as_deref() {
                let message = if launcher_running {
                    "检测到官方启动器已在运行，复用"
                } else {
                    "引擎已接管"
                };
                let status = launcher_win::ipc::StatusSnapshot::new(&game, "starting", message);
                let _ = launcher_win::ipc::publish(None, path, &status);
            }
            if parsed.hold_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(parsed.hold_ms));
            }
            if let Some(path) = status_path.as_deref() {
                let status =
                    launcher_win::ipc::StatusSnapshot::new(&game, "finished", "示例会话结束");
                let _ = launcher_win::ipc::publish(None, path, &status);
            }
            drop(guard);
            write_report(
                parsed.report.as_deref(),
                &format!(
                    "{{\n  \"code\": \"session-finished\",\n  \"reuse_launcher\": {launcher_running}\n}}\n"
                ),
            );
            Ok(EXIT_OK)
        }
        "restore" => Err(Failure::NotImplemented(
            "--engine restore 尚未实现（M2-06 之后接恢复流程）".to_owned(),
        )),
        other => Err(Failure::Usage(format!("未知子命令：--engine {other}"))),
    }
}

/// 失败路径也要能落盘：从命令行里再找一次 `--report`。
fn report_path_from_env() -> Option<PathBuf> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let index = arguments.iter().position(|value| value == "--report")?;
    arguments.get(index + 1).map(PathBuf::from)
}

fn write_report(path: Option<&std::path::Path>, text: &str) {
    let Some(path) = path else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, text);
}
