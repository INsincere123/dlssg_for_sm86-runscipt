//! 引擎流程规划（M8-01 第一步）：把"这个游戏要部署到哪、启动什么"算清楚。
//!
//! 只做定位与前置检查，不做任何写操作——`--dry-run` 直接输出这份计划，
//! 真实执行时才按同样的路径去调用 `resources` / `diagnostic` / `engine_config` / `spawn`。
//!
//! 路径约定（对齐现有 PowerShell 实现）：
//!
//! | 用途 | 路径 |
//! |---|---|
//! | 资源根 | `<渲染目录>\dlssg_sm86` |
//! | Engine.ini 原件 | `<项目目录>\Saved\Config\Windows\Engine.ini` |
//! | 明文配置源 | `<启动器根>\配置\<gameId>\Engine.ini` |
//! | 运行时副本 | `%TEMP%\dlssg_sm86-<gameId>-<pid>.ini` |
//! | 备份 | `%TEMP%\dlssg_sm86-<gameId>-<pid>.bak.ini` |
//! | 部署记录 | `<启动器根>\.state\<gameId>\active.json` |

use std::path::{Path, PathBuf};

use launcher_core::error::{Error, Result};
use launcher_core::preferences::GameProfile;
use launcher_core::resources::{Deployment, ProxyChoice};

/// 一次运行需要的全部路径与信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunPlan {
    pub game_id: String,
    pub game_name: String,
    /// 游戏根目录（用户填的那个）。
    pub game_directory: PathBuf,
    /// 实际渲染 EXE。
    pub render_executable: PathBuf,
    /// 实际启动的 EXE（官方启动器；未配置时等于渲染程序）。
    pub launch_executable: PathBuf,
    /// 官方启动器的进程名（用于等待它出现）；直接启动渲染程序时为 `None`。
    pub launcher_process_name: Option<String>,
    /// 已经有未完成的部署记录：接管现场，不再重复部署。
    pub adopted: bool,
    /// 渲染目录（资源与代理链接都落在这里）。
    pub render_directory: PathBuf,
    /// 项目目录（`Saved\Config` 所在的那一层）。
    pub project_directory: PathBuf,
    /// 游戏的 `Engine.ini` 原件。
    pub engine_path: PathBuf,
    /// 明文配置源。
    pub engine_source: PathBuf,
    /// 诊断 INI（`dlssg_sm86.ini`）的运行时副本（`%TEMP%`）。
    pub runtime_path: PathBuf,
    /// `Engine.ini` 的只读副本（`%TEMP%`，与诊断 INI 分开）。
    pub engine_runtime_path: PathBuf,
    pub backup_path: PathBuf,
    /// 部署记录位置。
    pub journal_path: PathBuf,
    pub proxy_index: u8,
    pub proxy_relative: String,
    pub launch_mode: String,
    pub app_id: Option<String>,
    /// 该游戏的 12 键设置（生成诊断 INI 用）。
    pub settings: launcher_core::settings::Settings,
    /// 启动参数（Steam 为 `-applaunch <AppID>`，Direct 一般为空）。
    pub launch_arguments: Vec<String>,
    /// 档案里声明的会话策略文本。
    pub session_policy: Option<String>,
}

/// 平台相关的输入（真实路径来自注册表/环境变量，测试里注入假值）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformPaths {
    /// 启动器根目录（资源、配置源、`.state` 都相对它）。
    pub launcher_root: PathBuf,
    /// `%TEMP%`。
    pub temp_directory: PathBuf,
    /// 当前进程号（用于临时文件名）。
    pub pid: u32,
}

/// 由偏好条目算出运行计划；任何前置条件不满足就报错，不做猜测。
pub fn plan(profile: &GameProfile, paths: &PlatformPaths) -> Result<RunPlan> {
    if profile.directory.as_os_str().is_empty() {
        return Err(Error::invalid(format!(
            "游戏「{}」还没有填写目录。",
            profile.name
        )));
    }
    if !profile.directory.is_dir() {
        return Err(Error::invalid(format!(
            "游戏目录不存在：{}",
            profile.directory.display()
        )));
    }
    let relative = profile.render_executable.as_deref().ok_or_else(|| {
        Error::invalid(format!(
            "游戏「{}」还没有确定渲染程序；请重新添加一次游戏。",
            profile.name
        ))
    })?;
    let context = launcher_core::path::PathContext::current()?;
    let render_executable = launcher_core::direct::resolve_executable(
        &context,
        &profile.directory,
        relative,
        "渲染程序",
    )?;
    if !render_executable.is_file() {
        return Err(Error::invalid(format!(
            "渲染程序不存在：{}",
            render_executable.display()
        )));
    }
    let render_directory = render_executable
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| Error::invalid("渲染程序没有父目录。"))?;

    // 项目目录：`<…>\Binaries\Win64\X.exe` → 上两级是项目目录；否则用渲染目录。
    let project_directory = project_of(&render_directory);

    // 明文配置源：档案里给了就用它（相对启动器根或绝对），否则退回约定路径。
    // Engine.ini 部署是可选项：没有声明配置目录（游戏不改配置）或关掉开关时，完全不动游戏配置，
    // 也就不需要配置源——早先在这里无条件要求配置源，导致"不加配置就没法启动"。
    let engine_directory = profile
        .config_dir
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let deploy_engine = profile.deploy_engine_ini && engine_directory.is_some();
    let engine_source = match profile.config_source.as_deref() {
        Some(value) if !value.trim().is_empty() => {
            let candidate = PathBuf::from(value.trim());
            if candidate.is_absolute() {
                candidate
            } else {
                paths.launcher_root.join(candidate)
            }
        }
        _ => default_engine_source(paths, profile),
    };
    if deploy_engine && !engine_source.is_file() {
        return Err(Error::invalid(format!(
            "缺少明文配置源：{}\n把一份 Engine.ini 放到这个路径，或在「画质」里用 05-自定义 指定一个已有文件；\
             如果这个游戏不需要改配置，把它的 config_dir 留空即可。",
            engine_source.display()
        )));
    }

    let launch_executable = match profile.launch_executable.as_deref() {
        Some(value) if !value.trim().is_empty() => {
            let candidate = PathBuf::from(value.trim());
            if candidate.is_absolute() {
                candidate
            } else {
                profile.directory.join(candidate)
            }
        }
        _ => render_executable.clone(),
    };
    let proxy = ProxyChoice::from_index(&profile.proxy_index.to_string())?;
    let stem = format!("dlssg_sm86-{}-{}", profile.id, paths.pid);
    // 代理 DLL 的**目标**必须存在：否则部署出悬空链接，任何导入该 DLL 的模块（含 ACE）都会以 126 失败。
    // 代理 DLL 的**目标**必须存在：否则部署出悬空链接，任何导入该 DLL 的模块
    // （包括反作弊自己的 ACE-Base64.dll）都会以 126（模块找不到）加载失败。
    let proxy_target = paths.launcher_root.join(proxy.relative());
    if !proxy_target.is_file() {
        return Err(Error::invalid(format!(
            "代理 DLL 目标不存在：{}（运行根需要 dlssg_sm86.ini、version.dll 与 alternatives\\）",
            proxy_target.display()
        )));
    }

    let plan = RunPlan {
        game_id: profile.id.clone(),
        game_name: profile.name.clone(),
        game_directory: profile.directory.clone(),
        adopted: false,
        launcher_process_name: if launch_executable == render_executable {
            None
        } else {
            Some(
                profile
                    .launcher_process_name
                    .clone()
                    .unwrap_or_else(|| process_name(&launch_executable)),
            )
        },
        launch_executable: launch_executable.clone(),
        render_executable: render_executable.clone(),
        render_directory: render_directory.clone(),
        project_directory: project_directory.clone(),
        engine_path: match (&profile.config_dir, deploy_engine) {
            (Some(directory), true) if !directory.trim().is_empty() => {
                PathBuf::from(directory.trim()).join("Engine.ini")
            }
            _ => PathBuf::new(),
        },
        engine_source,
        runtime_path: paths.temp_directory.join(format!("{stem}.ini")),
        engine_runtime_path: paths.temp_directory.join(format!("{stem}-engine.ini")),
        backup_path: paths.temp_directory.join(format!("{stem}.bak.ini")),
        journal_path: paths
            .launcher_root
            .join(".state")
            .join(&profile.id)
            .join("active.json"),
        proxy_index: profile.proxy_index,
        proxy_relative: proxy.relative().to_string_lossy().into_owned(),
        launch_mode: profile.launch_mode.clone(),
        app_id: profile.app_id.clone(),
        settings: profile.settings.clone().unwrap_or_default(),
        launch_arguments: profile.launch_arguments.clone().unwrap_or_default(),
        session_policy: profile.session_policy.clone(),
    };

    // 已有未完成的部署：接管而不是拒绝，交给上层展示"已部署"并继续监控。
    Ok(RunPlan {
        adopted: plan.journal_path.exists(),
        ..plan
    })
}

/// 部署前统一预检：一次列出**所有**缺失项，避免逐个步骤才暴露。
///
/// 只做存在性判断，不做修复；返回空表示可以部署。
pub fn preflight(plan: &RunPlan, launcher_root: &Path) -> Vec<String> {
    let mut missing = Vec::new();
    if !plan.render_executable.is_file() {
        missing.push(format!(
            "渲染程序不存在：{}",
            plan.render_executable.display()
        ));
    }
    if plan.launcher_process_name.is_some() && !plan.launch_executable.is_file() {
        missing.push(format!(
            "官方启动器不存在：{}",
            plan.launch_executable.display()
        ));
    }
    let proxy_target = launcher_root.join(&plan.proxy_relative);
    if !proxy_target.is_file() {
        missing.push(format!("代理 DLL 目标不存在：{}", proxy_target.display()));
    }
    for relative in [
        "dlssg_sm86.ini",
        r"dlssg_sm86\runtime\nvngx_dlssg.dll",
        r"dlssg_sm86\backend\sm86_backend.dll",
        r"dlssg_sm86\kernels\fg_kernels.dll",
        r"dlssg_sm86\xefg\xefg_kernels.dll",
    ] {
        let path = launcher_root.join(relative);
        if !path.is_file() {
            missing.push(format!("资源缺失：{}", path.display()));
        }
    }
    if !plan.engine_path.as_os_str().is_empty() {
        let parent_ok = plan.engine_path.parent().map(Path::is_dir).unwrap_or(false);
        if !parent_ok {
            missing.push(format!(
                "游戏配置目录不存在：{}",
                plan.engine_path.display()
            ));
        }
        if !plan.engine_source.is_file() {
            missing.push(format!(
                "明文配置源不存在：{}",
                plan.engine_source.display()
            ));
        }
    }
    missing
}
/// 资源视图（`Deployment` 需要借用生命周期；真实执行接线时会用到）。
#[allow(dead_code)]
pub fn deployment<'a>(
    plan: &'a RunPlan,
    launcher_root: &'a Path,
    proxy: ProxyChoice,
) -> Deployment<'a> {
    Deployment::new(launcher_root, &plan.render_directory, proxy)
}

/// 「结束并恢复」的结果（写成报告给界面读）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StopSummary {
    pub game_id: String,
    /// 已结束的进程号。
    pub killed: Vec<u32>,
    /// 结束失败的进程号（权限不足时会出现）。
    pub failed: Vec<u32>,
    /// 引擎卡住时被本命令结束。
    pub engine_stopped: bool,
    /// 是否完成了恢复。
    pub restored: bool,
    pub message: String,
}

impl StopSummary {
    pub fn to_json(&self) -> String {
        #[derive(serde::Serialize)]
        struct Report<'a> {
            code: &'a str,
            game: &'a str,
            killed: usize,
            failed: usize,
            restored: bool,
            engine_stopped: bool,
            message: &'a str,
        }
        let report = Report {
            code: "stopped",
            game: &self.game_id,
            killed: self.killed.len(),
            failed: self.failed.len(),
            restored: self.restored,
            engine_stopped: self.engine_stopped,
            message: &self.message,
        };
        let mut text = serde_json::to_string_pretty(&report)
            .unwrap_or_else(|_| "{\"code\":\"stopped\"}".to_owned());
        text.push('\n');
        text
    }
}

/// 渲染 EXE 的绝对路径（档案里存的是相对游戏目录的路径）。
pub fn render_path(profile: &GameProfile) -> Option<PathBuf> {
    let relative = profile.render_executable.as_deref()?.trim();
    if relative.is_empty() {
        return None;
    }
    let candidate = PathBuf::from(relative);
    Some(if candidate.is_absolute() {
        candidate
    } else {
        profile.directory.join(candidate)
    })
}

/// 官方启动器的绝对路径（没配置就返回 `None`）。
pub fn launcher_path(profile: &GameProfile) -> Option<PathBuf> {
    let declared = profile.launch_executable.as_deref()?.trim();
    if declared.is_empty() {
        return None;
    }
    let candidate = PathBuf::from(declared);
    Some(if candidate.is_absolute() {
        candidate
    } else {
        profile.directory.join(candidate)
    })
}

/// 该游戏"相关进程"的匹配条件：渲染 EXE、官方启动器入口、官方启动器进程名、
/// 以及渲染目录里跑的任何进程（二游的启动器会再起别的进程名）。
///
/// **Steam 游戏不能把 steam.exe 算进去**：那是用户自己的客户端，结束它会把整个 Steam 关掉。
pub fn game_processes(profile: &GameProfile) -> launcher_win::ProcessFilter {
    let steam = profile.launch_mode.eq_ignore_ascii_case("Steam");
    let mut executables: Vec<PathBuf> = Vec::new();
    if let Some(render) = render_path(profile) {
        executables.push(render);
    }
    if !steam {
        if let Some(launcher) = launcher_path(profile) {
            if !executables.iter().any(|path| path == &launcher) {
                executables.push(launcher);
            }
        }
    }
    let mut names: Vec<String> = Vec::new();
    if !steam {
        if let Some(name) = profile.launcher_process_name.as_deref() {
            let index = name.to_ascii_lowercase().rfind(".exe");
            let trimmed = index
                .map(|index| name[..index].to_owned())
                .unwrap_or_else(|| name.to_owned());
            if !trimmed.trim().is_empty() {
                names.push(trimmed);
            }
        }
    }
    // 目录条件：渲染 EXE 所在目录（部署与进程都在这里），退回游戏目录。
    let directory = render_path(profile)
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .or_else(|| Some(profile.directory.clone()))
        .filter(|path| path.is_dir());
    launcher_win::ProcessFilter {
        executables,
        names,
        directory,
    }
}

/// 该游戏是否有进程在跑（界面用它决定"启动"还是"结束并恢复"）。
pub fn is_game_running(profile: &GameProfile) -> bool {
    !launcher_win::matching_processes(&game_processes(profile)).is_empty()
}

/// 强行结束该游戏的相关进程。
pub fn terminate_game(profile: &GameProfile, wait: std::time::Duration) -> (Vec<u32>, Vec<u32>) {
    launcher_win::terminate_matching(&game_processes(profile), wait)
}

/// 杀完游戏后等引擎自己恢复的宽限时间：要盖住鸣潮 10 秒的热重启窗口，但别让用户干等。
const ENGINE_GRACE: std::time::Duration = std::time::Duration::from_secs(20);

/// 「结束并恢复」：先强行结束该游戏的进程，再按情况恢复。
///
/// 三种情况分别处理（都要先杀进程，否则恢复会撞上被占用/仍在写的文件）：
///
/// 1. **引擎还在跑**（它持着会话锁）：杀完进程它会自己走完会话并恢复，这里只等它把记录删掉
/// 2. **没有引擎但记录还在**（例如引擎异常退出）：由本进程补做恢复
/// 3. **没有记录**：杀进程就够了，不报错
///
/// 这个命令通常以管理员身份运行，因为游戏本身可能也是提权的。
pub fn stop(
    launcher_root: &Path,
    profile: &GameProfile,
    kill_wait: std::time::Duration,
) -> Result<StopSummary> {
    let mut summary = StopSummary {
        game_id: profile.id.clone(),
        ..StopSummary::default()
    };
    let targets = game_processes(profile);
    if !targets.is_empty() {
        let (killed, failed) = terminate_game(profile, kill_wait);
        summary.killed = killed;
        summary.failed = failed;
    }

    let pending =
        launcher_core::journal_store::load_for_game(launcher_root, &profile.id)?.is_some();
    if !pending {
        summary.message = format!(
            "已结束 {} 个进程；没有未完成的部署，无需恢复",
            summary.killed.len()
        );
        return Ok(summary);
    }

    if launcher_win::session_active() {
        // 引擎还活着：正常情况下杀完游戏它会自己结束会话并恢复，这里先等它把记录删掉。
        if wait_for_journal_removed(launcher_root, &profile.id, ENGINE_GRACE) {
            summary.restored = true;
            summary.message = format!("已结束 {} 个进程；引擎已完成恢复", summary.killed.len());
            return Ok(summary);
        }
        // 引擎自己卡住了（例如在等一个永远不会出现的启动器进程）：结束它再自己恢复。
        if !recover_from_stuck_engine(launcher_root, &profile.id)? {
            return Err(Error::invalid(
                "已结束游戏进程，但引擎在预期时间内没有完成恢复，也找不到可以结束的引擎进程；请查看日志。",
            ));
        }
        summary.engine_stopped = true;
    }

    let pending =
        launcher_core::journal_store::load_for_game(launcher_root, &profile.id)?.is_some();
    if !pending {
        summary.restored = true;
        summary.message = if summary.engine_stopped {
            format!(
                "已结束 {} 个进程，并结束了卡住的引擎；部署记录已不在，无需恢复",
                summary.killed.len()
            )
        } else {
            format!(
                "已结束 {} 个进程；部署记录已不在，无需恢复",
                summary.killed.len()
            )
        };
        return Ok(summary);
    }
    let message = restore_existing(launcher_root, &profile.id, false)?;
    summary.restored = true;
    summary.message = if summary.engine_stopped {
        format!(
            "已结束 {} 个进程，并结束了卡住的引擎；{message}",
            summary.killed.len()
        )
    } else {
        format!("已结束 {} 个进程；{message}", summary.killed.len())
    };
    Ok(summary)
}

/// 引擎自己卡住时：按部署记录里的 `ScriptPid` 结束它，等会话锁释放。
///
/// 返回是否真的结束了一个引擎进程。
fn recover_from_stuck_engine(launcher_root: &Path, game_id: &str) -> Result<bool> {
    let Some((_, journal)) = launcher_core::journal_store::load_for_game(launcher_root, game_id)?
    else {
        return Ok(false);
    };
    let pid = journal.script_pid;
    if pid == 0 || pid == std::process::id() {
        return Ok(false);
    }
    if !launcher_win::terminate_pid(pid, std::time::Duration::from_secs(5)) {
        return Ok(false);
    }
    // 等它把 Global\DlssgSession 释放掉，否则下面的恢复打不开部署会话。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while launcher_win::session_active() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    Ok(true)
}

/// 等部署记录消失（= 恢复完成）；超时返回 `false`。
fn wait_for_journal_removed(
    launcher_root: &Path,
    game_id: &str,
    timeout: std::time::Duration,
) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let pending = launcher_core::journal_store::load_for_game(launcher_root, game_id)
            .ok()
            .flatten()
            .is_some();
        if !pending {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

/// `--engine restore`：按已有部署记录手动恢复。
///
/// 要求游戏已退出（与 PowerShell 版一致）；记录不存在时直接报错而不是静默成功。
pub fn restore_existing(launcher_root: &Path, game_id: &str, dry_run: bool) -> Result<String> {
    use launcher_core::{journal_store, restore};

    let (path, journal) = journal_store::load_for_game(launcher_root, game_id)?
        .ok_or_else(|| Error::invalid(format!("没有找到部署记录：{game_id}（无需恢复）")))?;
    if dry_run {
        return Ok(format!("恢复计划：{}", path.display()));
    }
    let render = std::path::PathBuf::from(&journal.render);
    // 残留检查必须用**记录里的那个代理**：以前写死成 1（version.dll），
    // 于是用 winmm 部署的游戏在恢复后会被"version.dll 残留"误报（那个文件其实是别人的）。
    let recorded_proxy = journal
        .proxy_name
        .as_deref()
        .unwrap_or("version.dll")
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("version.dll")
        .to_owned();
    let proxy =
        ProxyChoice::from_name(&recorded_proxy).unwrap_or_else(|_| ProxyChoice::default_choice());
    let mut state = journal_store::DeploymentSession::open(
        &launcher_win::WinMutexOps,
        launcher_root,
        &render,
        journal,
    )?;
    let outcome = restore::restore(
        &mut state,
        &launcher_win::WinSymlinkOps,
        &restore::RestoreOptions {
            automatic: true,
            ..restore::RestoreOptions::default()
        },
        &|_stopped| Ok(()),
        &[],
    )?;
    let deployment = Deployment::new(launcher_root, &render, proxy);
    launcher_core::resources::assert_resources_removed(
        &deployment,
        None,
        None,
        &launcher_win::WinSymlinkOps,
    )?;
    Ok(format!(
        "恢复完成：{} 步，记录已删除（{}）",
        outcome.restored.len(),
        path.display()
    ))
}
/// 默认明文配置源：`<根>\配置\<id>\Engine.ini`，没有就试 `<根>\配置\<游戏名>\Engine.ini`
/// （便携包里配置目录按中文名放，与 `启动脚本\配置\异环` 同名）。都不存在时返回按 id 的路径，
/// 让报错信息指向约定位置。
fn default_engine_source(paths: &PlatformPaths, profile: &GameProfile) -> PathBuf {
    let root = paths.launcher_root.join("配置");
    let by_id = root.join(&profile.id).join("Engine.ini");
    if by_id.is_file() {
        return by_id;
    }
    let by_name = root.join(&profile.name).join("Engine.ini");
    if by_name.is_file() {
        return by_name;
    }
    by_id
}

/// `<…>\Binaries\Win64` 的上两级是项目目录；否则返回渲染目录本身。
fn project_of(render_directory: &Path) -> PathBuf {
    let name = render_directory
        .file_name()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let parent_name = render_directory
        .parent()
        .and_then(Path::file_name)
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if name == "win64" && parent_name == "binaries" {
        if let Some(project) = render_directory.parent().and_then(Path::parent) {
            return project.to_path_buf();
        }
    }
    render_directory.to_path_buf()
}

/// 会话结束判定：由游戏 id 决定策略（三款策略不得统一，见 PARITY N-1/R8）。
#[allow(dead_code)]
pub fn session_policy(game_id: &str) -> launcher_core::session::SessionPolicy {
    policy_from_profile(game_id, None)
}

/// 档案里的策略优先（`first-exit` / `wait-all` / `hot-restart:<秒>`），
/// 否则按游戏 id 推断——二游的差异应该写在档案里，不该靠 id 猜。
pub fn policy_from_profile(
    game_id: &str,
    declared: Option<&str>,
) -> launcher_core::session::SessionPolicy {
    use launcher_core::session::SessionPolicy;
    if let Some(text) = declared {
        let trimmed = text.trim().to_ascii_lowercase();
        if trimmed == "first-exit" {
            return SessionPolicy::FirstExit;
        }
        if trimmed == "wait-all" {
            return SessionPolicy::WaitAllTargets;
        }
        if let Some(seconds) = trimmed.strip_prefix("hot-restart:") {
            if let Ok(value) = seconds.trim().parse::<u64>() {
                return SessionPolicy::HotRestart {
                    grace: std::time::Duration::from_secs(value),
                };
            }
        }
    }
    match game_id {
        "wuthering" => SessionPolicy::wuthering(),
        "nte" => SessionPolicy::FirstExit,
        _ => SessionPolicy::WaitAllTargets,
    }
}

/// 进程名：去掉 `.exe` 后缀（与 `ProcessProbe` 的约定一致）。
pub fn process_name(executable: &Path) -> String {
    executable
        .file_stem()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| executable.to_string_lossy().into_owned())
}

/// 由计划构造会话规格：按游戏 id 选策略，按渲染程序名等目标进程。
pub fn session_spec(plan: &RunPlan) -> launcher_core::session::SessionSpec {
    let spec = launcher_core::session::SessionSpec::new(policy_from_profile(
        &plan.game_id,
        plan.session_policy.as_deref(),
    ))
    .with_game(process_name(&plan.render_executable));
    match plan.launcher_process_name.as_deref() {
        Some(name) => spec.with_launcher(name),
        None => spec,
    }
}
/// 一次执行的可选项。
#[derive(Debug, Clone, Copy, Default)]
pub struct RunOptions {
    /// 只走规划与既有状态检查，不写任何东西。
    pub dry_run: bool,
    /// 部署完成后直接恢复，不启动游戏（测试与自检用）。
    pub skip_launch: bool,
}

/// 执行结果（写成报告 JSON）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunSummary {
    pub game_id: String,
    pub proxy: String,
    pub resources: usize,
    pub diagnostic: bool,
    pub engine_copy: bool,
    pub launched: bool,
    pub restored: bool,
    pub render: PathBuf,
    pub engine: PathBuf,
    pub launch: PathBuf,
    pub steps: Vec<String>,
}

impl RunSummary {
    /// 报告 JSON：交给 serde_json 处理转义，避免手写替换导致反斜杠被转义错。
    pub fn to_json(&self) -> String {
        #[derive(serde::Serialize)]
        struct Report<'a> {
            code: &'a str,
            game: &'a str,
            proxy: &'a str,
            resources: usize,
            diagnostic: bool,
            engine_copy: bool,
            launched: bool,
            restored: bool,
            render: String,
            engine: String,
            launch: String,
            steps: &'a [String],
        }
        let report = Report {
            code: "executed",
            game: &self.game_id,
            proxy: &self.proxy,
            resources: self.resources,
            diagnostic: self.diagnostic,
            engine_copy: self.engine_copy,
            launched: self.launched,
            restored: self.restored,
            render: self.render.display().to_string(),
            engine: self.engine.display().to_string(),
            launch: self.launch.display().to_string(),
            steps: &self.steps,
        };
        let mut text = serde_json::to_string_pretty(&report)
            .unwrap_or_else(|_| "{\"code\":\"executed\"}".to_owned());
        text.push('\n');
        text
    }
}

/// 按计划执行（`dry_run` 只模拟，不落盘），随后恢复。
pub fn execute(plan: &RunPlan, launcher_root: &Path, options: &RunOptions) -> Result<RunSummary> {
    use launcher_core::{diagnostic, engine_config, journal, journal_store, resources, restore};

    let mut summary = RunSummary {
        game_id: plan.game_id.clone(),
        proxy: plan.proxy_relative.clone(),
        render: plan.render_executable.clone(),
        engine: plan.engine_path.clone(),
        launch: plan.launch_executable.clone(),
        ..RunSummary::default()
    };
    // 预检放在最前面：一次把缺的东西全列出来。
    let missing = preflight(plan, launcher_root);
    if !missing.is_empty() && !options.dry_run {
        return Err(Error::invalid(format!(
            "部署前预检未通过（{} 项）：\n{}",
            missing.len(),
            missing.join("\n")
        )));
    }
    summary.steps.push(if missing.is_empty() {
        "preflight:ok".to_owned()
    } else {
        format!("preflight:{}", missing.len())
    });

    let ops = launcher_win::WinSymlinkOps;
    let mutex = launcher_win::WinMutexOps;
    let proxy = ProxyChoice::from_index(&plan.proxy_index.to_string())?;
    let deployment = resources::Deployment::new(launcher_root, &plan.render_directory, proxy);

    // 起始记录：会话号 32 位小写十六进制，脚本启动时间用 PS 的 7 位小数格式。
    let seed = format!(
        "{}-{}-{}",
        plan.game_id,
        std::process::id(),
        journal_store::identity(&plan.render_directory)?
    );
    let session = launcher_core::path::sha256_upper(seed.as_bytes()).to_lowercase();
    let session = session[..32].to_owned();
    let now = std::time::SystemTime::now();
    let original_existed = plan.engine_path.is_file();
    let original_hash = launcher_core::path::file_hash_or_none(&plan.engine_path)?;
    let record = journal::Journal::initialize(
        &plan.game_id,
        &session,
        std::process::id(),
        journal_store::format_script_started(now),
        &plan.render_directory,
        &plan.engine_path,
        &plan.engine_runtime_path,
        &plan.backup_path,
        original_existed,
        original_hash,
        deployment.storage_dll().exists(),
        None,
        &plan.proxy_relative,
    );
    // dry-run 不写记录：只按计划模拟每一步。
    if options.dry_run {
        if plan.adopted {
            summary.steps.push("adopted".to_owned());
        }
        summary.steps.push("session:simulated".to_owned());
        summary.steps.push(format!("proxy:{}", plan.proxy_relative));
        summary.resources = deployment.resource_specs().len();
        summary
            .steps
            .push(format!("resources:{}", summary.resources));
        summary.diagnostic = true;
        summary.steps.push("diagnostic".to_owned());
        summary.engine_copy = !plan.engine_path.as_os_str().is_empty();
        summary.steps.push("engine.ini".to_owned());
        summary.steps.push("dry-run:stop".to_owned());
        return Ok(summary);
    }
    // 接手上一轮的记录是危险的：上一轮引擎可能被强杀，记录里可能是"部署到一半"的状态，
    // 而"接管"会整段跳过部署——那样游戏会在没有注入的情况下跑起来，用户只会看到"没生效"。
    // 所以先把遗留记录恢复掉，再按全新一轮部署（部署本身很快，可预测性更重要）。
    let mut adopted = plan.adopted;
    if adopted {
        restore_existing(launcher_root, &plan.game_id, false).map_err(|error| {
            Error::invalid(format!(
                "发现上一轮的遗留部署，先恢复它时失败：{error}\n\
                 请先点该行的「恢复原状」处理完再来启动。"
            ))
        })?;
        adopted = false;
    }

    let mut state = if adopted {
        let (_, existing) = journal_store::load_for_game(launcher_root, &plan.game_id)?
            .ok_or_else(|| Error::invalid("接管失败：找不到部署记录".to_owned()))?;
        journal_store::DeploymentSession::open(
            &mutex,
            launcher_root,
            &plan.render_directory,
            existing,
        )?
    } else {
        journal_store::DeploymentSession::open(
            &mutex,
            launcher_root,
            &plan.render_directory,
            record,
        )?
    };
    summary.steps.push("session:open".to_owned());

    // 日志：与现有实现同目录（`<根>\logs`）；写失败不影响主流程。
    let mut log =
        launcher_core::log::LauncherLog::open(launcher_root, "engine", now, std::process::id())
            .ok();
    let note = |log: &mut Option<launcher_core::log::LauncherLog>, message: &str| {
        if let Some(log) = log.as_mut() {
            let _ = log.write(launcher_core::log::Level::Info, message);
        }
    };
    note(
        &mut log,
        &format!(
            "游戏 {} 渲染程序 {}",
            plan.game_id,
            plan.render_executable.display()
        ),
    );

    // 二游：先拉起官方启动器，等它出现后再部署（部署完成即注入生效）。
    // Steam 不走这条：它的启动顺序是"先部署 → 再 steam -applaunch → 等游戏"。
    if plan.launcher_process_name.is_some()
        && !plan.launch_mode.eq_ignore_ascii_case("Steam")
        && !options.dry_run
        && !options.skip_launch
    {
        let child = launcher_win::spawn_process(
            &plan.launch_executable,
            &plan.launch_arguments,
            Some(&plan.game_directory),
            false,
        )?;
        child.forget();
        summary.launched = true;
        let name = plan.launcher_process_name.clone().unwrap_or_default();
        note(&mut log, &format!("已拉起官方启动器：{name}"));
        summary.steps.push(format!("launcher:{name}"));
        // 等启动器进程出现：先按名字认，再退一步按"游戏目录里有进程在跑"认
        // （鸣潮的入口是 NTELauncher.exe，真正的启动器进程叫 NTEGame.exe）。
        //
        // 只等 10 秒：启动器要不要出现、什么时候出现由它自己决定（异环的启动器常常
        // 起一下就退，更新器另起进程），而用户登录+点开始游戏至少要几十秒。等太久只会
        // 让"点启动没反应"，反而错过注入窗口。超时照常部署，日志留一条说明。
        let mut seen = false;
        for _ in 0..50 {
            let spec = launcher_core::session::SessionSpec::new(
                launcher_core::session::SessionPolicy::FirstExit,
            )
            .with_launcher(name.clone());
            let by_name = launcher_core::session::launcher_already_running(
                &launcher_win::WinProcessProbe,
                &spec,
            )
            .unwrap_or(false);
            let by_path = !launcher_win::processes_in_directory(&plan.game_directory).is_empty();
            if by_name || by_path {
                summary.steps.push(if by_name {
                    "launcher:seen".to_owned()
                } else {
                    "launcher:seen-in-dir".to_owned()
                });
                seen = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        if !seen {
            summary.steps.push("launcher:not-seen".to_owned());
            note(
                &mut log,
                "10 秒内没看到官方启动器进程，直接部署（游戏启动前注入才有效）",
            );
        }
    }
    // 部署四步：任何一步失败都要尽力回滚，避免把游戏留在半部署状态。
    // 接管已有部署时跳过，避免重复部署覆盖现场。
    let deployed: Result<()> = (|| {
        if adopted {
            return Ok(());
        }
        // 1) 代理链接
        resources::install_proxy_link(&mut state, &ops, &deployment, options.dry_run)?;
        summary.steps.push(format!("proxy:{}", plan.proxy_relative));

        // 2) 资源目录链接
        let installed =
            resources::install_resources(&mut state, &ops, &deployment, options.dry_run)?;
        summary.resources = installed.created.len();
        summary
            .steps
            .push(format!("resources:{}", installed.created.len()));

        // 3) 诊断 INI（`dlssg_sm86.ini` 链接到 %TEMP% 副本）
        let identity = journal_store::identity(&plan.render_directory)?;
        let text = plan.settings.render(&identity)?;
        let diagnostic_spec = diagnostic::DiagnosticSpec {
            render_dir: plan.render_directory.clone(),
            temp_path: plan.runtime_path.clone(),
            text,
        };
        if plan.runtime_path.as_os_str().is_empty() || plan.render_directory.as_os_str().is_empty()
        {
            return Err(Error::invalid(format!(
                "运行计划缺少路径：runtime={:?} render={:?} engine={:?} source={:?}",
                plan.runtime_path, plan.render_directory, plan.engine_path, plan.engine_source
            )));
        }
        let deployed_diagnostic =
            diagnostic::deploy_diagnostic(&mut state, &ops, &diagnostic_spec, options.dry_run)?;
        summary.diagnostic = deployed_diagnostic.temp_written || options.dry_run;
        summary.steps.push("diagnostic".to_owned());

        // 4) Engine.ini 只读副本 + 备份 + 链接
        let marker = if plan.game_id == "wuthering" {
            launcher_core::engine_config::RuntimeMarker::Wuthering
        } else if plan.game_id == "nte" {
            launcher_core::engine_config::RuntimeMarker::Neverness
        } else if let Some(app_id) = &plan.app_id {
            launcher_core::engine_config::RuntimeMarker::Steam {
                app_id: app_id.clone(),
            }
        } else {
            launcher_core::engine_config::RuntimeMarker::Direct {
                game_id: plan.game_id.clone(),
            }
        };
        let engine_spec = engine_config::EngineConfigSpec {
            engine_path: plan.engine_path.clone(),
            runtime_path: plan.engine_runtime_path.clone(),
            backup_path: plan.backup_path.clone(),
            source_path: plan.engine_source.clone(),
            marker,
        };
        // 没有声明配置目录时不碰游戏配置（也不要求配置源存在）。
        if plan.engine_path.as_os_str().is_empty() {
            summary.steps.push("engine.ini:skipped".to_owned());
        } else {
            if plan.engine_runtime_path.as_os_str().is_empty()
                || plan.backup_path.as_os_str().is_empty()
            {
                return Err(Error::invalid(
                    "Engine.ini 部署路径不完整（runtime/backup 有空值）".to_owned(),
                ));
            }
            let prepared = engine_config::prepare_engine_config(
                &mut state,
                &ops,
                &engine_spec,
                options.dry_run,
            )?;
            summary.engine_copy = prepared.runtime_written || options.dry_run;
            summary.steps.push("engine.ini".to_owned());
        }
        note(
            &mut log,
            &format!(
                "部署完成：代理 {}，资源 {} 个",
                plan.proxy_relative, summary.resources
            ),
        );

        Ok(())
    })();
    if let Err(error) = deployed {
        let step = summary
            .steps
            .last()
            .cloned()
            .unwrap_or_else(|| "start".to_owned());
        let _ = restore::restore(
            &mut state,
            &ops,
            &restore::RestoreOptions {
                automatic: true,
                ..restore::RestoreOptions::default()
            },
            &|_stopped| Ok(()),
            &[],
        );
        return Err(Error::invalid(format!("部署步骤 {step} 失败：{error}")));
    }

    // 5) 启动 + 会话等待
    // 二游：官方启动器已经在部署前拉起（见上面的 "launcher:" 步骤），这里只等会话。
    if options.skip_launch {
        summary.steps.push("launch:skipped".to_owned());
    } else {
        // Steam 游戏要**先部署再让 Steam 拉起**（游戏进程一启动就加载代理 DLL，
        // 部署晚了注入就错过窗口）；二游走的是"先起官方启动器再部署"那条路。
        if plan.launcher_process_name.is_none() || plan.launch_mode.eq_ignore_ascii_case("Steam") {
            let child = launcher_win::spawn_process(
                &plan.launch_executable,
                &plan.launch_arguments,
                Some(&plan.game_directory),
                false,
            )?;
            child.forget();
            summary.launched = true;
            summary
                .steps
                .push(format!("launch:{}", process_name(&plan.launch_executable)));
        }

        let spec = session_spec(plan);
        let mut machine =
            launcher_core::session::SessionMachine::new(spec.clone(), std::time::SystemTime::now());
        let outcome = launcher_core::session::run_session(
            &mut machine,
            &spec,
            &launcher_win::WinProcessProbe,
            &launcher_win::SystemClock,
            &|duration| std::thread::sleep(duration),
        )?;
        note(&mut log, &format!("会话结束：{outcome:?}"));
        summary.steps.push(format!("session:{outcome:?}"));
    }

    // 6) 恢复 + 残留检查
    let outcome = restore::restore(
        &mut state,
        &ops,
        &restore::RestoreOptions::default(),
        &|_stopped| Ok(()),
        &[],
    )?;
    summary.restored = outcome.is_complete();
    summary
        .steps
        .push(format!("restore:{}", outcome.restored.len()));
    resources::assert_resources_removed(
        &deployment,
        Some(&plan.engine_runtime_path),
        Some(&plan.backup_path),
        &ops,
    )?;
    summary.steps.push("verify:clean".to_owned());
    note(&mut log, "恢复完成，残留检查通过");
    if let Some(log) = log.as_ref() {
        summary.steps.push(format!("log:{}", log.path().display()));
    }
    state.finish()?;
    Ok(summary)
}
#[cfg(test)]
mod tests {
    use super::*;
    use launcher_core::preferences::GameProfile;

    struct Tree(PathBuf);

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tree(name: &str) -> Tree {
        let root = std::env::temp_dir().join(format!("dlssg-plan-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("根目录");
        Tree(root)
    }

    fn write(root: &Path, relative: &str, content: &[u8]) {
        let path = root.join(relative.replace('/', "\\"));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("父目录");
        }
        std::fs::write(&path, content).expect("写文件");
    }

    /// 造一个 UE 风格的游戏目录 + 启动器根，返回 (游戏目录, 启动器根, 平台输入)
    fn fixture(name: &str) -> (Tree, PathBuf, PathBuf, PlatformPaths) {
        let sandbox = tree(name);
        let root = sandbox.0.join("启动器");
        let game = sandbox.0.join("游戏");
        write(
            &game,
            "Project/Binaries/Win64/Game-Win64-Shipping.exe",
            b"MZ",
        );
        write(&root, "配置/wuthering/Engine.ini", b"[SystemSettings]\r\n");
        write(&root, "version.dll", b"stub");
        write(&root, "dlssg_sm86.ini", b"stub");
        let paths = PlatformPaths {
            launcher_root: root.clone(),
            temp_directory: sandbox.0.join("temp"),
            pid: 4242,
        };
        std::fs::create_dir_all(&paths.temp_directory).expect("temp");
        (sandbox, game, root, paths)
    }

    fn profile(game: &Path) -> GameProfile {
        let mut profile = GameProfile::new("wuthering", "鸣潮", game.to_path_buf());
        profile.config_dir = Some(
            game.join(r"Project\Saved\Config\Windows")
                .to_string_lossy()
                .into_owned(),
        );
        profile.render_executable =
            Some(r"Project\Binaries\Win64\Game-Win64-Shipping.exe".to_owned());
        profile
    }

    #[test]
    fn steam_games_never_treat_the_steam_client_as_their_process() {
        // Steam 入口：结束进程时绝不能把用户自己的 Steam 关掉。
        let mut steam = GameProfile::new("steam-4115450", "PBZ", PathBuf::from(r"C:\PBZ"));
        steam.launch_mode = "Steam".to_owned();
        steam.render_executable = Some(r"platform\steam\PBZ.exe".to_owned());
        steam.launch_executable = Some(r"C:\Program Files (x86)\Steam\Steam.exe".to_owned());
        steam.launcher_process_name = Some("steam.exe".to_owned());
        let filter = game_processes(&steam);
        assert!(
            filter
                .executables
                .iter()
                .all(|path| !path.to_string_lossy().to_lowercase().ends_with("steam.exe")),
            "不能把 steam.exe 当作目标：{:?}",
            filter.executables
        );
        assert!(
            filter.names.is_empty(),
            "也不能按名字 steam 匹配：{:?}",
            filter.names
        );
        assert!(
            filter
                .executables
                .iter()
                .any(|path| path.to_string_lossy().ends_with(r"platform\steam\PBZ.exe")),
            "渲染 EXE 仍要在列表里"
        );

        // 二游：官方启动器与它真正的进程名都要算进去。
        let mut nte = GameProfile::new("nte", "异环", PathBuf::from(r"C:\Games\NTE"));
        nte.launch_mode = "Direct".to_owned();
        nte.render_executable =
            Some(r"Client\WindowsNoEditor\HT\Binaries\Win64\HTGame.exe".to_owned());
        nte.launch_executable = Some(r"C:\Games\NTE\NTELauncher.exe".to_owned());
        nte.launcher_process_name = Some("NTEGame.exe".to_owned());
        let filter = game_processes(&nte);
        assert_eq!(filter.names, vec!["NTEGame".to_owned()]);
        assert!(
            filter
                .executables
                .iter()
                .any(|path| path.to_string_lossy().ends_with("NTELauncher.exe"))
        );
    }

    #[test]
    fn render_and_launcher_paths_follow_the_profile() {
        let mut profile = GameProfile::new("wuthering", "鸣潮", PathBuf::from(r"C:\Games\WuWa"));
        profile.render_executable =
            Some(r"Client\Binaries\Win64\Client-Win64-Shipping.exe".to_owned());
        profile.launch_executable =
            Some(r"C:\Program Files\Wuthering Waves\launcher.exe".to_owned());
        assert_eq!(
            render_path(&profile),
            Some(PathBuf::from(
                r"C:\Games\WuWa\Client\Binaries\Win64\Client-Win64-Shipping.exe"
            ))
        );
        assert_eq!(
            launcher_path(&profile),
            Some(PathBuf::from(
                r"C:\Program Files\Wuthering Waves\launcher.exe"
            )),
            "绝对路径的启动器要原样使用"
        );

        let mut direct = GameProfile::new("x", "X", PathBuf::from(r"C:\X"));
        direct.render_executable = Some("Game.exe".to_owned());
        assert!(launcher_path(&direct).is_none(), "没有启动器就是 None");
    }

    #[test]
    fn plan_resolves_all_paths_for_a_ue_game() {
        let (_sandbox, game, root, paths) = fixture("ue");
        let plan = plan(&profile(&game), &paths).expect("规划");

        assert_eq!(plan.game_id, "wuthering");
        assert_eq!(plan.render_directory, game.join(r"Project\Binaries\Win64"));
        assert_eq!(plan.project_directory, game.join("Project"));
        // 档案里声明了 config_dir → Engine.ini 就在该目录下
        assert_eq!(
            plan.engine_path,
            game.join(r"Project\Saved\Config\Windows\Engine.ini")
        );
        assert_eq!(plan.engine_source, root.join(r"配置\wuthering\Engine.ini"));
        assert_eq!(
            plan.runtime_path,
            paths.temp_directory.join("dlssg_sm86-wuthering-4242.ini")
        );
        assert_eq!(
            plan.backup_path,
            paths
                .temp_directory
                .join("dlssg_sm86-wuthering-4242.bak.ini")
        );
        assert_eq!(
            plan.journal_path,
            root.join(r".state\wuthering\active.json")
        );
        assert_eq!(plan.proxy_index, 1);
        assert_eq!(plan.proxy_relative, "version.dll");
    }

    #[test]
    fn missing_directory_or_source_is_refused() {
        let (_sandbox, game, root, paths) = fixture("missing");
        let mut broken = profile(&game);
        broken.directory = game.join("nope");
        let error = plan(&broken, &paths).expect_err("目录不存在应报错");
        assert!(error.to_string().contains("游戏目录不存在"), "{error}");

        let mut no_source = profile(&game);
        no_source.id = "neverness".to_owned();
        let error = plan(&no_source, &paths).expect_err("缺配置源应报错");
        assert!(error.to_string().contains("缺少明文配置源"), "{error}");
        assert!(!root.join("配置/neverness").exists());
    }

    #[test]
    fn existing_journal_is_adopted_not_rejected() {
        let (_sandbox, game, root, paths) = fixture("journal");
        std::fs::create_dir_all(root.join(r".state\wuthering")).expect("state");
        std::fs::write(root.join(r".state\wuthering\active.json"), b"{}").expect("记录");
        // 有记录时不再拒绝：标记为"接管"，由上层展示已部署并继续监控。
        let plan = plan(&profile(&game), &paths).expect("有记录也应能规划");
        assert!(plan.adopted, "应标记为接管");
        assert!(plan.journal_path.exists());
    }

    #[test]
    fn project_directory_falls_back_to_the_render_directory() {
        assert_eq!(
            project_of(Path::new(r"D:\Game\bin")),
            PathBuf::from(r"D:\Game\bin")
        );
        assert_eq!(
            project_of(Path::new(r"D:\Game\Project\Binaries\Win64")),
            PathBuf::from(r"D:\Game\Project")
        );
        assert_eq!(
            project_of(Path::new(r"D:\Game\Other\Win64")),
            PathBuf::from(r"D:\Game\Other\Win64")
        );
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;

    #[test]
    fn policy_follows_the_game_id_and_never_unifies_strategies() {
        use launcher_core::session::SessionPolicy;
        assert!(matches!(
            session_policy("wuthering"),
            SessionPolicy::HotRestart { .. }
        ));
        assert!(matches!(session_policy("nte"), SessionPolicy::FirstExit));
        assert!(matches!(
            session_policy("steam-4356430"),
            SessionPolicy::WaitAllTargets
        ));
        assert!(matches!(
            session_policy("direct-0123456789abcdef0123456789abcdef"),
            SessionPolicy::WaitAllTargets
        ));
    }

    #[test]
    fn process_name_strips_the_exe_suffix() {
        assert_eq!(
            process_name(Path::new(r"D:\Game\Client-Win64-Shipping.exe")),
            "Client-Win64-Shipping"
        );
        assert_eq!(process_name(Path::new(r"D:\Game\Game")), "Game");
    }

    #[test]
    fn steam_games_launch_with_applaunch_arguments() {
        let arguments = launcher_core::steam::launch_arguments("4356430").expect("参数");
        assert_eq!(
            arguments,
            vec!["-applaunch".to_owned(), "4356430".to_owned()]
        );
    }
}
