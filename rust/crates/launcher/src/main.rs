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
mod gpu;
mod handoff;
mod icon;
mod rename;
mod scan;
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
            report_failure(message);
            ExitCode::from(code)
        }
    }
}

enum Failure {
    Usage(String),
    #[allow(dead_code)]
    NotImplemented(String),
}

/// 调试用的界面预览（`--ui-preview`）：不连引擎，直接把某个界面摆出来。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preview {
    /// 恢复成功的弹窗。
    RestoreOk,
    /// 恢复失败的弹窗。
    RestoreFail,
    /// 帧生成设置页。
    FrameGen,
    /// 画质设置页。
    Quality,
    /// 扫描结果窗口（示例数据）。
    Scan,
}

impl Preview {
    fn parse(value: &str) -> Result<Self, Failure> {
        match value {
            "restore-ok" => Ok(Self::RestoreOk),
            "restore-fail" => Ok(Self::RestoreFail),
            "framegen" => Ok(Self::FrameGen),
            "quality" => Ok(Self::Quality),
            "scan" => Ok(Self::Scan),
            other => Err(Failure::Usage(format!(
                "--ui-preview 只接受 restore-ok / restore-fail / framegen / quality / scan，收到：{other}"
            ))),
        }
    }

    /// 恢复结果窗的内容（示例文本，只用于核对排版）。
    pub fn restore_outcome(self) -> Option<(bool, &'static str, &'static str)> {
        match self {
            Self::RestoreOk => Some((true, "恢复原状完成", "已恢复 5 项（示例）。")),
            Self::RestoreFail => Some((
                false,
                "恢复原状失败",
                "备份校验失败（示例）。部署记录与备份已保留。",
            )),
            _ => None,
        }
    }

    /// 要打开哪个设置视图；`None` 表示不开设置页。
    pub fn settings_view(self) -> Option<settings_form::SettingsView> {
        match self {
            Self::FrameGen => Some(settings_form::SettingsView::FrameGen),
            Self::Quality => Some(settings_form::SettingsView::Quality),
            _ => None,
        }
    }

    /// 扫描结果窗口的示例数据（只用于核对排版）。
    pub fn scan_sample(self) -> Option<scan::ScanResult> {
        if self != Self::Scan {
            return None;
        }
        let mut nte = launcher_core::preferences::GameProfile::new(
            "nte",
            "异环",
            std::path::PathBuf::from(r"C:\Games\Neverness To Everness"),
        );
        nte.render_executable =
            Some(r"Client\WindowsNoEditor\HT\Binaries\Win64\HTGame.exe".to_owned());
        nte.proxy_index = 2;
        let mut wukong = launcher_core::preferences::GameProfile::new(
            "steam-2358720",
            "黑神话：悟空",
            std::path::PathBuf::from(r"D:\SteamLibrary\steamapps\common\BlackMythWukong"),
        );
        wukong.launch_mode = "Steam".to_owned();
        wukong.render_executable = Some(r"b1\Binaries\Win64\b1-Win64-Shipping.exe".to_owned());
        Some(scan::ScanResult {
            ready: vec![
                scan::ReadyGame {
                    profile: nte,
                    source: "注册表卸载项「异环」".to_owned(),
                },
                scan::ReadyGame {
                    profile: wukong,
                    source: "Steam 库 D:\\SteamLibrary".to_owned(),
                },
            ],
            ambiguous: vec![
                scan::AmbiguousGame {
                    name: "NBA 2K27".to_owned(),
                    directory: std::path::PathBuf::from(
                        r"D:\SteamLibrary\steamapps\common\NBA 2K27",
                    ),
                    source: "Steam 库 D:\\SteamLibrary".to_owned(),
                    reason: "有 2 个候选，需要手动确认渲染程序".to_owned(),
                },
                scan::AmbiguousGame {
                    name: "Risk of Rain 2".to_owned(),
                    directory: std::path::PathBuf::from(
                        r"D:\SteamLibrary\steamapps\common\Risk of Rain 2",
                    ),
                    source: "Steam 库 D:\\SteamLibrary".to_owned(),
                    reason: "没看到虚幻打包结构或自带 DLSS-G 组件".to_owned(),
                },
            ],
            notes: vec![
                "没在注册表里认出「鸣潮」的安装目录，需要的话手动「添加游戏」".to_owned(),
                "没有 GOG 记录".to_owned(),
            ],
        })
    }
}

struct Arguments {
    engine: Option<String>,
    report: Option<PathBuf>,
    game: Option<String>,
    ui_request_id: Option<String>,
    elevate: bool,
    hold_ms: u64,
    launcher_name: Option<String>,
    dry_run: bool,
    skip_launch: bool,
    root: Option<PathBuf>,
    preferences: Option<PathBuf>,
    /// 显卡名称切换子命令（`apply` / `restore` / `reset-state` / `status`）。
    gpu: Option<String>,
    /// 显卡名称切换的目标名（`--gpu apply` 用）。
    name: Option<String>,
    /// 测试辅助：什么都不做，睡这么多毫秒后退出（扮演"假游戏进程"）。
    sleep_ms: u64,
}

impl Arguments {
    fn ui_request(&self) -> Option<handoff::Request> {
        Some(handoff::Request {
            id: self.ui_request_id.clone()?,
            game: self.game.clone()?,
            command: handoff::Command::parse(self.engine.as_deref()?)?,
        })
    }
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
        ui_request_id: None,
        elevate: false,
        hold_ms: 0,
        launcher_name: None,
        dry_run: false,
        skip_launch: false,
        root: None,
        preferences: None,
        gpu: None,
        name: None,
        sleep_ms: 0,
    };
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--engine" => {
                parsed.engine = Some(next_value(arguments, index, "--engine")?);
                index += 2;
            }
            "--gpu" => {
                parsed.gpu = Some(next_value(arguments, index, "--gpu")?);
                index += 2;
            }
            "--name" => {
                parsed.name = Some(next_value(arguments, index, "--name")?);
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
            "--ui-request" => {
                let id = next_value(arguments, index, "--ui-request")?;
                if !launcher_core::journal::is_valid_session(&id) || parsed.ui_request_id.is_some()
                {
                    return Err(Failure::Usage("交接请求 ID 无效。".to_owned()));
                }
                parsed.ui_request_id = Some(id);
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
    if parsed.ui_request_id.is_some()
        && (parsed.ui_request().is_none() || parsed.report.is_none() || parsed.gpu.is_some())
    {
        return Err(Failure::Usage(
            "交接请求需要引擎操作、游戏 ID 与报告路径。".to_owned(),
        ));
    }
    if parsed.ui_request_id.is_some() && (parsed.dry_run || parsed.skip_launch) {
        return Err(Failure::Usage(
            "正式交接请求不能使用模拟运行参数。".to_owned(),
        ));
    }
    Ok(parsed)
}

fn run() -> Result<u8, Failure> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    // 图形入口自己解析参数，绝不落到引擎/显卡的参数校验里。
    if !arguments
        .iter()
        .any(|value| value == "--engine" || value == "--version" || value == "--gpu")
    {
        return run_gui(&arguments);
    }
    let parsed = parse(&arguments)?;
    if let Some(action) = parsed.gpu.clone() {
        return gpu_command(&parsed, &action);
    }
    let Some(command) = parsed.engine.as_deref() else {
        return Err(Failure::Usage(
            "用法：--engine check|run|restore [--game <id>] [--report <json>]；\
             或 --gpu status|apply|restore|reset-state [--name <名称>] [--report <json>]"
                .to_owned(),
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
            // 隔离会话测试用本程序模拟有一定寿命的渲染进程。
            if parsed.sleep_ms != 0 {
                std::thread::sleep(std::time::Duration::from_millis(parsed.sleep_ms));
            }
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
        "stop" => engine_stop(&parsed),
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
            let preferences = engine_preferences(&parsed, &root).ok();
            let profile = preferences
                .as_ref()
                .and_then(|preferences| preferences.find(&game));
            let message = engine::restore_existing(&root, &game, parsed.dry_run, profile)
                .map_err(|error| Failure::Usage(format!("恢复失败：{error}")))?;
            let report = serde_json::json!({
                "code": "restored",
                "game": game,
                "message": message,
            });
            let report = serde_json::to_string_pretty(&report)
                .map_err(|error| Failure::Usage(format!("生成恢复报告失败：{error}")))?;
            write_engine_result(&parsed, handoff::Outcome::Completed { message }, &report)?;
            Ok(EXIT_OK)
        }
        other => Err(Failure::Usage(format!("未知子命令：--engine {other}"))),
    }
}

/// `--gpu <动作>`：显卡名称切换。
///
/// **成功与失败都要写报告**：这个子进程是 `windows` 子系统、提权后没有控制台，
/// 界面（和被拉起的父进程）只能从报告里拿结果。退出码只是给命令行排查用的。
fn gpu_command(parsed: &Arguments, action: &str) -> Result<u8, Failure> {
    let Some(kind) = gpu::Action::parse(action) else {
        return Err(Failure::Usage(format!(
            "未知的 --gpu 子命令：{action}（可用：status / apply / restore / reset-state）"
        )));
    };
    let report = gpu::execute(kind, parsed.name.as_deref());
    write_report(parsed.report.as_deref(), &report.to_json());
    Ok(if report.ok {
        EXIT_OK
    } else {
        EXIT_CHILD_FAILED
    })
}

/// `--engine stop`：强行结束该游戏的进程，然后（必要时）恢复原状。
///
/// 界面点「结束并恢复」时以管理员身份调用它：游戏本身可能也是提权的，
/// 非提权进程既杀不掉也读不到它的路径。
fn engine_stop(parsed: &Arguments) -> Result<u8, Failure> {
    let game = parsed
        .game
        .clone()
        .ok_or_else(|| Failure::Usage("--engine stop 需要 --game".to_owned()))?;
    let root = parsed.root.clone().unwrap_or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
            .unwrap_or_default()
    });
    let preferences = engine_preferences(parsed, &root)
        .map_err(|error| Failure::Usage(format!("读不到偏好：{error}")))?;
    let profile = preferences
        .find(&game)
        .ok_or_else(|| Failure::Usage(format!("偏好里没有这个游戏：{game}")))?;

    let summary = engine::stop(&root, profile, std::time::Duration::from_secs(8))
        .map_err(|error| Failure::Usage(format!("结束并恢复失败：{error}")))?;
    let report = summary.to_json();
    write_engine_result(
        parsed,
        handoff::Outcome::Completed {
            message: summary.message,
        },
        &report,
    )?;
    Ok(EXIT_OK)
}

fn engine_preferences(
    parsed: &Arguments,
    root: &std::path::Path,
) -> Result<launcher_core::preferences::Preferences, String> {
    // 偏好跟随启动器安装位置。
    let install_base = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_else(|| root.to_path_buf());
    let preferences_file = parsed
        .preferences
        .clone()
        .or_else(|| launcher_core::prefs::preferences_path(&install_base).ok());
    load_preferences(preferences_file.as_deref())
}

/// 读偏好：文件不存在算空，读到了但解析失败要**报出来**。
///
/// 以前这里 `.ok()` 一把吞掉：偏好文件被手改坏之后，界面/引擎只会说
/// "偏好里没有这个游戏"，排查时完全看不出真正原因。
fn load_preferences(
    path: Option<&std::path::Path>,
) -> Result<launcher_core::preferences::Preferences, String> {
    let Some(path) = path else {
        return Ok(launcher_core::preferences::Preferences::default());
    };
    if !path.exists() {
        return Ok(launcher_core::preferences::Preferences::default());
    }
    launcher_core::preferences::load(path).map_err(|error| format!("{}：{error}", path.display()))
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
            write_engine_result(
                parsed,
                handoff::Outcome::SessionBusy {
                    message: "另一个游戏会话正在运行。".to_owned(),
                },
                "{\n  \"code\": \"session-busy\",\n  \"message\": \"另一个游戏会话正在运行\"\n}\n",
            )?;
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

    // 共享状态供重新打开窗口时查找会话。启动接管使用独立的请求报告。
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
        } else if parsed.ui_request_id.is_some() {
            "正在准备游戏部署"
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
    let preferences = load_preferences(preferences_file.as_deref())
        .map_err(|error| Failure::Usage(format!("读不到偏好：{error}")))?;
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
    let summary = engine::execute(&plan, &root, &options, || {
        if let (Some(request), Some(report)) = (parsed.ui_request(), parsed.report.as_deref()) {
            if let Err(error) = handoff::publish(&request, report, handoff::Outcome::Accepted) {
                eprintln!("无法写入接管确认：{error}");
            }
        }
    })
    .map_err(|error| Failure::Usage(format!("执行失败：{error}")))?;
    write_engine_result(
        parsed,
        handoff::Outcome::Completed {
            message: "会话已结束，部署已恢复。".to_owned(),
        },
        &summary.to_json(),
    )?;
    drop(guard);
    Ok(EXIT_OK)
}

/// 图形入口（M4）：单实例 + 主窗口；`--ui-selftest <ms> <report>` 用于验收。
fn run_gui(arguments: &[String]) -> Result<u8, Failure> {
    let mut self_test = ui::SelfTest::default();
    let mut preview: Option<Preview> = None;
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
            // 调试：不连引擎，直接把某个界面摆出来（截图核对用）。
            "--ui-preview" => {
                let value = next_value(arguments, index, "--ui-preview")?;
                preview = Some(Preview::parse(&value)?);
                index += 2;
            }
            other => return Err(Failure::Usage(format!("未知参数：{other}"))),
        }
    }

    // 图形入口自己的单实例：与引擎会话锁分开，避免"界面开着"就挡住引擎。
    let single =
        launcher_win::acquire_named_session("Local\\DlssgLauncherUi", std::time::Duration::ZERO)
            .map_err(|error| Failure::Usage(format!("界面单实例不可用：{error}")))?;
    let Some(_single) = single else {
        // 已经有界面在跑：常规做法是把它的窗口叫到最前面，然后本进程安静退出。
        // 命名事件让运行中的界面走"托盘里点显示窗口"的同一套恢复路径（隐藏到托盘、
        // 交接后隐藏、最小化都能还原），直接找窗口只是兜底。
        launcher_win::activate::request_activation(ui::WINDOW_TITLE);
        return Ok(EXIT_OK);
    };
    // 界面拿到锁之后再建激活事件：后来者能可靠找到它。
    let activation = launcher_win::activate::ActivationEvent::create().ok();

    let root = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_default();
    let mut view = view_model::LauncherView::new();
    // 运行根：偏好里指定了就用它（资源可能在另一个目录），否则用 exe 所在目录。
    let mut run_root = root.clone();
    let preferences_path = launcher_core::prefs::preferences_path(&root).ok();
    match load_preferences(preferences_path.as_deref()) {
        Err(error) => {
            // 偏好坏了就明说，别让它"看起来一个游戏都没有"。
            view.notice = format!("偏好读不出来（列表会是空的）：{error}");
        }
        Ok(preferences) => {
            if let Some(declared) = preferences.launcher_root.as_deref() {
                if !declared.trim().is_empty() {
                    run_root = PathBuf::from(declared.trim());
                }
            }
            // 每一行的状态只看本行事实：本行的游戏进程在跑就是"运行中"，
            // 别的游戏在跑不能写进这一行（真机反馈过这个 bug）。
            view.session_taken = launcher_win::session_active();
            use launcher_core::ops::ProcessProbe;
            let processes = launcher_win::WinProcessProbe.list().unwrap_or_default();
            for game in &preferences.games {
                let pending = launcher_core::journal_store::load_for_game(&run_root, &game.id)
                    .ok()
                    .flatten();
                let running = engine::is_game_running(
                    Some(game),
                    pending.as_ref().map(|(_, journal)| journal),
                    &processes,
                );
                view.upsert(view_model::GameEntry {
                    id: game.id.clone(),
                    name: game.name.clone(),
                    directory: game.directory.clone(),
                    status: view_model::derive_status(
                        game.directory.is_dir(),
                        pending.is_some(),
                        running,
                    ),
                    engine_host: game.engine_host.clone(),
                    running,
                });
            }
        }
    }
    if !view.games.is_empty() {
        view.select(0);
    }

    // 日志保留 7 天：只清 <运行根>\logs\*.log，报告 JSON 与其它文件不动。
    let logs = about::logs_directory(&run_root);
    let pruned = launcher_core::log_retention::prune(
        &logs,
        std::time::SystemTime::now(),
        launcher_core::log_retention::RETENTION_DAYS,
    );
    if !pruned.removed.is_empty() || !pruned.errors.is_empty() {
        view.notice = pruned.summary();
    }

    let mut app = ui::LauncherApp::new(view, (self_test.hold_ms > 0).then_some(self_test))
        .with_preferences(launcher_core::prefs::preferences_path(&root).ok())
        .with_root(run_root.clone())
        .with_activation(activation);
    if let Some(preview) = preview {
        app = app.with_preview(preview);
    }
    let result = eframe::run_native(
        ui::WINDOW_TITLE,
        ui::native_options(ui::WINDOW_TITLE),
        Box::new(move |cc| {
            if let Some(font) = ui::font_data() {
                let mut fonts = egui::FontDefinitions::default();
                fonts
                    .font_data
                    .insert("msyh".to_owned(), std::sync::Arc::new(font));
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

/// 失败报告：交给 serde_json 转义。
///
/// 手写 `format!` 会把路径里的反斜杠当成 JSON 转义（`\p` 之类），界面显示时反斜杠全丢，
/// 排查时看不出到底是哪个文件（2026-11-23 真机截图暴露）。
fn error_report(message: &str) -> String {
    #[derive(serde::Serialize)]
    struct Report<'a> {
        error: &'a str,
    }
    let mut text = serde_json::to_string_pretty(&Report { error: message })
        .unwrap_or_else(|_| "{\"error\":\"报告序列化失败\"}".to_owned());
    text.push('\n');
    text
}

fn write_engine_result(
    parsed: &Arguments,
    outcome: handoff::Outcome,
    legacy: &str,
) -> Result<(), Failure> {
    if let (Some(request), Some(path)) = (parsed.ui_request(), parsed.report.as_deref()) {
        return handoff::publish(&request, path, outcome)
            .map_err(|error| Failure::Usage(format!("写入本次交接结果失败：{error}")));
    }
    write_report(parsed.report.as_deref(), legacy);
    Ok(())
}

/// 失败路径也要落盘。保留普通 CLI 结果格式，并给 UI 请求绑定身份。
fn report_failure(message: &str) {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = arguments
        .windows(2)
        .find(|pair| pair[0] == "--report")
        .map(|pair| PathBuf::from(&pair[1]))
    else {
        return;
    };
    if let Some((request, path)) = parse(&arguments)
        .ok()
        .and_then(|parsed| Some((parsed.ui_request()?, parsed.report?)))
    {
        let _ = handoff::publish(
            &request,
            &path,
            handoff::Outcome::Failed {
                message: message.to_owned(),
            },
        );
    } else {
        write_report(Some(&path), &error_report(message));
    }
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
