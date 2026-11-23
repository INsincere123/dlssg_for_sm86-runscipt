//! `DlssgLauncher`：单 EXE 入口（GUI + `--engine` 引擎）。
//!
//! - 没有 `--engine` / `--version` 时走图形入口（M4）
//! - `--engine check|run|restore` 走引擎（M2/M8）
//!
//! 整个二进制是 `windows` 子系统，没有控制台；产品输出一律写 `--report` 指定的 JSON。

#![windows_subsystem = "windows"]

mod about;
mod add_game;
mod engine;
mod handoff;
mod settings_form;
mod tray;
mod ui;
mod view_model;

use eframe::egui;
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
            // 没有控制台：消息写 stderr（多半被丢弃），主要靠退出码与报告文件表达。
            let (code, message) = match &failure {
                Failure::Usage(message) => (EXIT_USAGE, message),
                Failure::NotImplemented(message) => (EXIT_NOT_IMPLEMENTED, message),
            };
            eprintln!("{message}");
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
    #[allow(dead_code)]
    NotImplemented(String),
}

struct Arguments {
    engine: Option<String>,
    report: Option<PathBuf>,
    game: Option<String>,
    elevate: bool,
    hold_ms: u64,
    launcher_name: Option<String>,
    dry_run: bool,
    skip_launch: bool,
    root: Option<PathBuf>,
    preferences: Option<PathBuf>,
    /// 测试辅助：什么都不做，睡这么多毫秒后退出（扮演"假游戏进程"）。
    sleep_ms: u64,
}

fn next_value(arguments: &[String], index: usize, flag: &str) -> Result<String, Failure> {
    arguments
        .get(index + 1)
        .cloned()
        .ok_or_else(|| Failure::Usage(format!("{flag} 缺少值")))
}

fn parse(arguments: &[String]) -> Result<Arguments, Failure> {
    let mut parsed = Arguments {
        engine: None,
        report: None,
        game: None,
        elevate: false,
        hold_ms: 0,
        launcher_name: None,
        dry_run: false,
        skip_launch: false,
        root: None,
        preferences: None,
        sleep_ms: 0,
    };
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--engine" => {
                parsed.engine = Some(next_value(arguments, index, "--engine")?);
                index += 2;
            }
            "--report" => {
                parsed.report = Some(PathBuf::from(next_value(arguments, index, "--report")?));
                index += 2;
            }
            "--game" => {
                parsed.game = Some(next_value(arguments, index, "--game")?);
                index += 2;
            }
            "--root" => {
                parsed.root = Some(PathBuf::from(next_value(arguments, index, "--root")?));
                index += 2;
            }
            "--preferences" => {
                parsed.preferences = Some(PathBuf::from(next_value(
                    arguments,
                    index,
                    "--preferences",
                )?));
                index += 2;
            }
            "--launcher-name" => {
                parsed.launcher_name = Some(next_value(arguments, index, "--launcher-name")?);
                index += 2;
            }
            "--hold-ms" => {
                parsed.hold_ms = next_value(arguments, index, "--hold-ms")?
                    .parse()
                    .map_err(|_| Failure::Usage("--hold-ms 需要非负整数".to_owned()))?;
                index += 2;
            }
            "--sleep-ms" => {
                parsed.sleep_ms = next_value(arguments, index, "--sleep-ms")?
                    .parse()
                    .map_err(|_| Failure::Usage("--sleep-ms 需要非负整数".to_owned()))?;
                index += 2;
            }
            "--dry-run" => {
                parsed.dry_run = true;
                index += 1;
            }
            "--skip-launch" => {
                parsed.skip_launch = true;
                index += 1;
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
    // 图形入口自己解析参数，绝不落到引擎的参数校验里。
    if !arguments
        .iter()
        .any(|value| value == "--engine" || value == "--version")
    {
        return run_gui(&arguments);
    }
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
        "check" if parsed.elevate => check_elevated(&parsed),
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
        "run" => engine_run(&parsed),
        "restore" => {
            let game = parsed
                .game
                .clone()
                .ok_or_else(|| Failure::Usage("--engine restore 需要 --game".to_owned()))?;
            let root = parsed.root.clone().unwrap_or_else(|| {
                std::env::current_exe()
                    .ok()
                    .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
                    .unwrap_or_default()
            });
            let message = engine::restore_existing(&root, &game, parsed.dry_run)
                .map_err(|error| Failure::Usage(format!("恢复失败：{error}")))?;
            write_report(
                parsed.report.as_deref(),
                &format!(
                    "{{\n  \"code\": \"restored\",\n  \"game\": \"{game}\",\n  \"message\": \"{}\"\n}}\n",
                    message.replace('"', "'")
                ),
            );
            Ok(EXIT_OK)
        }
        other => Err(Failure::Usage(format!("未知子命令：--engine {other}"))),
    }
}

/// `--engine check --elevate`：自我提权跑一次 check，记录子进程结果。
fn check_elevated(parsed: &Arguments) -> Result<u8, Failure> {
    let child_report = parsed
        .report
        .as_ref()
        .map(|path| path.with_file_name("elevated-child.json"))
        .ok_or_else(|| Failure::Usage("--elevate 需要 --report".to_owned()))?;
    let executable = std::env::current_exe()
        .map_err(|error| Failure::Usage(format!("取自身路径失败：{error}")))?;
    let arguments = format!("--engine check --report \"{}\"", child_report.display());
    let outcome = launcher_win::elevate::run_elevated(
        &executable,
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
    Ok(match outcome {
        launcher_win::elevate::ElevationOutcome::Finished { code: 0 } => EXIT_OK,
        launcher_win::elevate::ElevationOutcome::Finished { .. } => EXIT_CHILD_FAILED,
        launcher_win::elevate::ElevationOutcome::Cancelled => EXIT_ELEVATION_CANCELLED,
    })
}

/// `--engine run`：单会话 → 状态发布 → 规划 → 执行（部署 → 启动/等待 → 恢复）。
fn engine_run(parsed: &Arguments) -> Result<u8, Failure> {
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

    let root = parsed.root.clone().unwrap_or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
            .unwrap_or_default()
    });

    // 状态先发布：界面/父进程能立刻看到"引擎已接管"或"复用官方启动器"。
    if let Some(path) = parsed
        .report
        .as_ref()
        .map(|report| report.with_file_name("status.json"))
    {
        let launcher_running = parsed
            .launcher_name
            .as_deref()
            .map(|name| {
                let spec = launcher_core::session::SessionSpec::new(
                    launcher_core::session::SessionPolicy::FirstExit,
                )
                .with_game(&game)
                .with_launcher(name);
                launcher_core::session::launcher_already_running(
                    &launcher_win::WinProcessProbe,
                    &spec,
                )
                .unwrap_or(false)
            })
            .unwrap_or(false);
        let message = if launcher_running {
            "检测到官方启动器已在运行，复用"
        } else {
            "引擎已接管"
        };
        let status = launcher_win::ipc::StatusSnapshot::new(&game, "starting", message);
        let _ = launcher_win::ipc::publish(None, &path, &status);
    }

    // 测试用：占住会话锁，模拟"另一个游戏正在跑"。
    if parsed.hold_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(parsed.hold_ms));
    }

    // 偏好跟着"启动器安装位置"走，不跟 `--root`（资源根）走：
    // 否则换个资源根就会读到另一个空的偏好目录，表现为"偏好里没有这个游戏"。
    let install_base = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_else(|| root.clone());
    let preferences_file = parsed
        .preferences
        .clone()
        .or_else(|| launcher_core::prefs::preferences_path(&install_base).ok());
    let preferences = preferences_file
        .as_deref()
        .and_then(|path| launcher_core::preferences::load(path).ok())
        .unwrap_or_default();
    let profile = preferences
        .find(&game)
        .cloned()
        .ok_or_else(|| Failure::Usage(format!("偏好里没有这个游戏：{game}")))?;

    let paths = engine::PlatformPaths {
        launcher_root: root.clone(),
        temp_directory: std::env::temp_dir(),
        pid: std::process::id(),
    };
    let plan = engine::plan(&profile, &paths)
        .map_err(|error| Failure::Usage(format!("运行计划失败：{error}")))?;

    let options = engine::RunOptions {
        dry_run: parsed.dry_run,
        skip_launch: parsed.skip_launch,
    };
    let summary = engine::execute(&plan, &root, &options)
        .map_err(|error| Failure::Usage(format!("执行失败：{error}")))?;
    write_report(parsed.report.as_deref(), &summary.to_json());
    drop(guard);
    Ok(EXIT_OK)
}

/// 图形入口（M4）：单实例 + 主窗口；`--ui-selftest <ms> <report>` 用于验收。
fn run_gui(arguments: &[String]) -> Result<u8, Failure> {
    let mut self_test = ui::SelfTest::default();
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--ui-selftest" => {
                self_test.hold_ms = next_value(arguments, index, "--ui-selftest")?
                    .parse()
                    .map_err(|_| Failure::Usage("--ui-selftest 需要毫秒数".to_owned()))?;
                self_test.report = Some(PathBuf::from(next_value(
                    arguments,
                    index + 1,
                    "--ui-selftest",
                )?));
                index += 3;
            }
            "--open-add-game" => index += 1,
            other => return Err(Failure::Usage(format!("未知参数：{other}"))),
        }
    }

    // 图形入口自己的单实例：与引擎会话锁分开，避免"界面开着"就挡住引擎。
    let single =
        launcher_win::acquire_named_session("Local\\DlssgLauncherUi", std::time::Duration::ZERO)
            .map_err(|error| Failure::Usage(format!("界面单实例不可用：{error}")))?;
    let Some(_single) = single else {
        return Ok(EXIT_OK);
    };

    let root = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_default();
    let mut view = view_model::LauncherView::new();
    // 运行根：偏好里指定了就用它（资源可能在另一个目录），否则用 exe 所在目录。
    let mut run_root = root.clone();
    if let Ok(path) = launcher_core::prefs::preferences_path(&root) {
        if let Ok(preferences) = launcher_core::preferences::load(&path) {
            if let Some(declared) = preferences.launcher_root.as_deref() {
                if !declared.trim().is_empty() {
                    run_root = PathBuf::from(declared.trim());
                }
            }
            let session_active = launcher_win::session_active();
            for game in &preferences.games {
                let pending = launcher_core::journal_store::load_for_game(&root, &game.id)
                    .ok()
                    .flatten()
                    .is_some();
                view.upsert(view_model::GameEntry {
                    id: game.id.clone(),
                    name: game.name.clone(),
                    directory: game.directory.clone(),
                    status: view_model::derive_status(
                        game.directory.is_dir(),
                        pending,
                        session_active,
                    ),
                    engine_host: game.engine_host.clone(),
                });
            }
        }
        if !view.games.is_empty() {
            view.select(0);
        }
    }

    let app = ui::LauncherApp::new(view, (self_test.hold_ms > 0).then_some(self_test))
        .with_preferences(launcher_core::prefs::preferences_path(&root).ok())
        .with_root(run_root.clone());
    let result = eframe::run_native(
        "DLSSG 启动器",
        ui::native_options("DLSSG 启动器"),
        Box::new(move |cc| {
            if let Some(bytes) = ui::font_bytes() {
                let mut fonts = egui::FontDefinitions::default();
                fonts.font_data.insert(
                    "msyh".to_owned(),
                    std::sync::Arc::new(egui::FontData::from_owned(bytes)),
                );
                for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                    fonts
                        .families
                        .entry(family)
                        .or_default()
                        .insert(0, "msyh".to_owned());
                }
                cc.egui_ctx.set_fonts(fonts);
            }
            Ok(Box::new(app))
        }),
    );
    result
        .map(|_| EXIT_OK)
        .map_err(|error| Failure::Usage(format!("窗口创建失败：{error}")))
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
