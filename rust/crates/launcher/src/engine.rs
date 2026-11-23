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
    // Engine.ini 部署是可选项：关掉就完全不动游戏配置。
    let deploy_engine = profile.deploy_engine_ini;
    let engine_source = match profile.config_source.as_deref() {
        Some(value) if !value.trim().is_empty() => {
            let candidate = PathBuf::from(value.trim());
            if candidate.is_absolute() {
                candidate
            } else {
                paths.launcher_root.join(candidate)
            }
        }
        _ => paths
            .launcher_root
            .join("配置")
            .join(&profile.id)
            .join("Engine.ini"),
    };
    if deploy_engine && !engine_source.is_file() {
        return Err(Error::invalid(format!(
            "缺少明文配置源：{}",
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
    let deployment = Deployment::new(launcher_root, &render, ProxyChoice::from_index("1")?);
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
    let mut state = if plan.adopted {
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
    if plan.launcher_process_name.is_some() && !options.dry_run && !options.skip_launch {
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
        // 等启动器进程出现（不设上限：出现即部署，没出现也照常部署以免用户白等）。
        for _ in 0..600 {
            let spec = launcher_core::session::SessionSpec::new(
                launcher_core::session::SessionPolicy::FirstExit,
            )
            .with_launcher(name.clone());
            if launcher_core::session::launcher_already_running(
                &launcher_win::WinProcessProbe,
                &spec,
            )
            .unwrap_or(false)
            {
                summary.steps.push("launcher:seen".to_owned());
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }
    // 部署四步：任何一步失败都要尽力回滚，避免把游戏留在半部署状态。
    // 接管已有部署时跳过，避免重复部署覆盖现场。
    let deployed: Result<()> = (|| {
        if plan.adopted {
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
        // 空路径意味"不做 Engine.ini 部署"；任何写操作前先自检，避免空路径 IO。
        if !plan.engine_path.as_os_str().is_empty()
            && (plan.engine_path.as_os_str().is_empty()
                || plan.engine_runtime_path.as_os_str().is_empty()
                || plan.backup_path.as_os_str().is_empty())
        {
            return Err(Error::invalid(
                "Engine.ini 部署路径不完整（engine/runtime/backup 有空值）".to_owned(),
            ));
        }
        let prepared =
            engine_config::prepare_engine_config(&mut state, &ops, &engine_spec, options.dry_run)?;
        summary.engine_copy = prepared.runtime_written || options.dry_run;
        summary.steps.push("engine.ini".to_owned());
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
        if plan.launcher_process_name.is_none() {
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
