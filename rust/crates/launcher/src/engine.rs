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
    /// 负载根：正式包里是 `<根>\bin`，旧布局就是启动器根。
    pub resource_root: PathBuf,
    /// 实际渲染 EXE。
    pub render_executable: PathBuf,
    /// 实际启动的 EXE（官方启动器；未配置时等于渲染程序）。
    pub launch_executable: PathBuf,
    /// 官方启动器的进程名（用于等待它出现）；直接启动渲染程序时为 `None`。
    pub launcher_process_name: Option<String>,
    /// 发现尚未恢复的部署记录。
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
    /// 启动参数（Steam 为 `-applaunch <AppID>`，Epic 为空）。
    pub launch_arguments: Vec<String>,
    /// 平台启动 URI（Epic）：由系统交给 Epic 启动器，不经过 EXE。
    pub launch_uri: Option<String>,
    /// 档案里声明的会话策略文本。
    pub session_policy: Option<String>,
}

/// 平台相关的输入（真实路径来自注册表/环境变量，测试里注入假值）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformPaths {
    /// 启动器根目录（配置源、`logs`、`.state` 都相对它）。
    pub launcher_root: PathBuf,
    /// `%TEMP%`。
    pub temp_directory: PathBuf,
    /// 当前进程号（用于临时文件名）。
    pub pid: u32,
}

/// 负载根：正式包里 DLL 与 INI 放在 `<根>\bin`，exe 留在根目录。
///
/// `bin` 存在就用它，不存在就退回启动器根——旧布局（资源直接摊在根目录）继续能用。
pub fn resource_root_of(launcher_root: &Path) -> PathBuf {
    let bin = launcher_root.join("bin");
    if bin.is_dir() {
        bin
    } else {
        launcher_root.to_path_buf()
    }
}

/// 按记录中代理链接的目标反推原负载根，兼容只保存代理文件名的旧记录。
fn journal_resource_root(
    journal: &launcher_core::journal::Journal,
    proxy: ProxyChoice,
) -> Option<PathBuf> {
    use launcher_core::journal::OperationKind;

    let relative = proxy.relative();
    let links = journal
        .operations
        .iter()
        .filter(|operation| operation.kind == OperationKind::Link)
        .map(|operation| (&operation.path, &operation.target))
        .chain(journal.reused.iter().map(|link| (&link.path, &link.target)));
    links
        .filter_map(|(path, target)| {
            let name = Path::new(path).file_name()?.to_str()?;
            if !name.eq_ignore_ascii_case(proxy.name()) {
                return None;
            }
            let target = Path::new(target);
            let root = target.ancestors().nth(relative.components().count())?;
            let expected = root.join(&relative);
            expected
                .to_str()?
                .replace('/', "\\")
                .eq_ignore_ascii_case(&target.to_str()?.replace('/', "\\"))
                .then(|| root.to_path_buf())
        })
        .next()
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

    // Steam 档案必须真的由 Steam 拉起。以前缺 `launch_executable` 时会退回"直接起渲染 EXE"：
    // 那等于绕开 Steam，Steamworks 游戏（影之刃零）还会自己要求 Steam 重启，部署会在真正
    // 在跑的进程出现之前就被恢复——日志写着"部署完成"，游戏却没注入。这种档案现在直接拒绝，
    // 缺 Steam.exe 时按注册表补一次。
    let steam_mode = profile.launch_mode.eq_ignore_ascii_case("Steam");
    // Epic 同理：由 Epic 启动器按 URI 拉起，绝不直接起渲染 EXE。
    let epic_mode = profile.launch_mode.eq_ignore_ascii_case("Epic");
    let declared_launch = profile
        .launch_executable
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let launch_executable = if steam_mode {
        let path = match declared_launch {
            Some(value) => {
                let candidate = PathBuf::from(value);
                if candidate.is_absolute() {
                    candidate
                } else {
                    profile.directory.join(candidate)
                }
            }
            None => launcher_win::registry::steam_install_directory()
                .map(|steam| steam.join("Steam.exe"))
                .filter(|candidate| candidate.is_file())
                .ok_or_else(|| {
                    Error::invalid(format!(
                        "游戏「{}」是 Steam 启动，但档案里没有 Steam.exe，注册表里也没找到 Steam。\
                         请确认 Steam 已安装，或删掉这条重新「添加游戏」。",
                        profile.name
                    ))
                })?,
        };
        if !path.is_file() {
            return Err(Error::invalid(format!(
                "游戏「{}」是 Steam 启动，但启动程序不存在：{}\n\
                 Steam 游戏必须由 Steam 拉起，不能直接启动渲染 EXE。",
                profile.name,
                path.display()
            )));
        }
        path
    } else if epic_mode {
        // Epic 由启动器 URI 拉起，不存在"启动 EXE"这一步。
        render_executable.clone()
    } else {
        match declared_launch {
            Some(value) => {
                let candidate = PathBuf::from(value);
                if candidate.is_absolute() {
                    candidate
                } else {
                    profile.directory.join(candidate)
                }
            }
            None => render_executable.clone(),
        }
    };
    // Steam 的启动参数只有一个来源：AppID。档案里没写参数时按 AppID 现算，
    // 免得旧档案留着空参数被当成"直接启动"。
    let launch_arguments = if steam_mode {
        let app_id = profile
            .app_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                Error::invalid(format!(
                    "游戏「{}」是 Steam 启动，但档案里没有 AppID；请删掉这条重新「添加游戏」，\
                     并选 Steam 库里的游戏目录。",
                    profile.name
                ))
            })?;
        launcher_core::steam::launch_arguments(app_id).map_err(|error| {
            Error::invalid(format!("游戏「{}」的 AppID 无效：{error}", profile.name))
        })?
    } else {
        profile.launch_arguments.clone().unwrap_or_default()
    };
    // Epic 的启动方式是协议 URI；AppName 只从档案来（扫描/添加时取自 Epic 清单）。
    let launch_uri = if epic_mode {
        let app_name = profile
            .epic_app_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                Error::invalid(format!(
                    "游戏「{}」是 Epic 启动，但档案里没有 AppName；请删掉这条重新「添加游戏」，\
                     并选 Epic 库里的游戏目录。",
                    profile.name
                ))
            })?;
        Some(
            launcher_core::platform::epic_launch_uri(app_name).map_err(|error| {
                Error::invalid(format!(
                    "游戏「{}」的 Epic AppName 无效：{error}",
                    profile.name
                ))
            })?,
        )
    } else {
        None
    };
    let proxy = ProxyChoice::from_index(&profile.proxy_index.to_string())?;
    let stem = format!("dlssg_sm86-{}-{}", profile.id, paths.pid);
    // 负载根：正式包里 DLL/INI 在 `<根>\bin`（`bin` 不存在时退回根，旧布局照旧）。
    let resource_root = resource_root_of(&paths.launcher_root);
    // 代理 DLL 的**目标**必须存在：否则部署出悬空链接，任何导入该 DLL 的模块
    // （包括反作弊自己的 ACE-Base64.dll）都会以 126（模块找不到）加载失败。
    let proxy_target = resource_root.join(proxy.relative());
    if !proxy_target.is_file() {
        return Err(Error::invalid(format!(
            "代理 DLL 目标不存在：{}\n\
             运行根里需要 dlssg_sm86.ini、version.dll 与 alternatives\\（正式包放在 bin\\ 子目录，旧布局放根目录）。",
            proxy_target.display()
        )));
    }

    let journal_relative = launcher_core::journal_store::state_relative_path(&profile.id);
    let mut journal_path = paths.launcher_root.join(&journal_relative);
    let mut adopted = false;
    for root in launcher_core::journal_store::state_roots(&paths.launcher_root) {
        let candidate = root.join(&journal_relative);
        match std::fs::symlink_metadata(&candidate) {
            Ok(_) => {
                journal_path = candidate;
                adopted = true;
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(Error::io(&candidate, error)),
        }
    }

    Ok(RunPlan {
        game_id: profile.id.clone(),
        game_name: profile.name.clone(),
        game_directory: profile.directory.clone(),
        resource_root: resource_root.clone(),
        adopted,
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
        journal_path,
        proxy_index: profile.proxy_index,
        proxy_relative: proxy.relative().to_string_lossy().into_owned(),
        launch_mode: profile.launch_mode.clone(),
        app_id: profile.app_id.clone(),
        settings: profile.settings.clone().unwrap_or_default(),
        launch_arguments,
        launch_uri,
        session_policy: profile.session_policy.clone(),
    })
}

/// 部署前统一预检：一次列出**所有**缺失项，避免逐个步骤才暴露。
///
/// 只做存在性判断，不做修复；返回空表示可以部署。
pub fn preflight(plan: &RunPlan) -> Vec<String> {
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
    let proxy_target = plan.resource_root.join(&plan.proxy_relative);
    if !proxy_target.is_file() {
        missing.push(format!("代理 DLL 目标不存在：{}", proxy_target.display()));
    }
    for relative in launcher_core::resources::CORE_REQUIRED {
        let path = plan.resource_root.join(relative);
        if !path.is_file() {
            missing.push(format!("资源缺失：{}", path.display()));
        }
    }
    // 超分开着才要求 sr 内核；轻量版（没有 sr 目录）在这条路上给出明确出路。
    if plan.settings.requires_super_resolution_resources() {
        for relative in launcher_core::resources::SUPER_RESOLUTION_REQUIRED {
            let path = plan.resource_root.join(relative);
            if !path.is_file() {
                missing.push(format!(
                    "缺超分内核：{}\n\
                     （轻量版不带 sr；要用超分请换全量包，或把 SuperResolution.Mode 设为 Off）",
                    path.display()
                ));
            }
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
pub fn deployment<'a>(plan: &'a RunPlan, proxy: ProxyChoice) -> Deployment<'a> {
    Deployment::new(&plan.resource_root, &plan.render_directory, proxy)
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
    // Epic 也一样：EpicGamesLauncher.exe 是用户自己的客户端，不能算成游戏进程。
    let platform_launched = steam || profile.launch_mode.eq_ignore_ascii_case("Epic");
    let mut executables: Vec<PathBuf> = Vec::new();
    if let Some(render) = render_path(profile) {
        executables.push(render);
    }
    if !platform_launched {
        if let Some(launcher) = launcher_path(profile) {
            if !executables.iter().any(|path| path == &launcher) {
                executables.push(launcher);
            }
        }
    }
    let mut names: Vec<String> = Vec::new();
    if !platform_launched {
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
    use launcher_core::journal_store;

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
    // 负载根优先按记录反推（换过布局也能找到当初那份资源），推不出来再用约定规则。
    let resource_root =
        journal_resource_root(&journal, proxy).unwrap_or_else(|| resource_root_of(launcher_root));
    let state = journal_store::DeploymentSession::resume(
        &launcher_win::WinMutexOps,
        path.clone(),
        journal,
    )?;
    let deployment = Deployment::new(&resource_root, &render, proxy);
    let outcome = restore_deployment(state, &deployment, true)?;
    Ok(format!(
        "恢复完成：{} 步，记录已删除（{}）",
        outcome.restored.len(),
        path.display()
    ))
}

/// 恢复已记录操作，持锁核对残留，最后删除实际记录。
fn restore_deployment(
    mut state: launcher_core::journal_store::DeploymentSession<'_>,
    deployment: &Deployment<'_>,
    automatic: bool,
) -> Result<launcher_core::restore::RestoreOutcome> {
    use launcher_core::{resources, restore};

    let ops = launcher_win::WinSymlinkOps;
    let mut allowed_links: Vec<_> = deployment
        .all_specs()
        .into_iter()
        .map(|spec| (spec.path, spec.target))
        .collect();
    allowed_links.extend(
        state
            .journal()
            .operations_of(launcher_core::journal::OperationKind::Diagnostic)
            .into_iter()
            .map(|operation| {
                (
                    deployment.render_dir.join("dlssg_sm86.ini"),
                    PathBuf::from(&operation.path),
                )
            }),
    );
    let outcome = restore::restore(
        &mut state,
        &ops,
        &restore::RestoreOptions {
            automatic,
            ..restore::RestoreOptions::default()
        },
        &|_| Ok(()),
        &allowed_links,
    )?;
    let journal = state.journal();
    resources::assert_resources_removed(
        deployment,
        (!journal.runtime.is_empty()).then(|| Path::new(&journal.runtime)),
        (!journal.backup.is_empty()).then(|| Path::new(&journal.backup)),
        &ops,
    )?;
    state.finish()?;
    Ok(outcome)
}

fn rollback_error(
    state: launcher_core::journal_store::DeploymentSession<'_>,
    deployment: &Deployment<'_>,
    stage: &str,
    error: Error,
) -> Error {
    let record = state.path().to_path_buf();
    match restore_deployment(state, deployment, true) {
        Ok(_) => Error::invalid(format!("{stage}失败：{error}\n部署已回滚，残留检查通过。")),
        Err(rollback) => Error::invalid(format!(
            "{stage}失败：{error}\n部署回滚失败：{rollback}\n请保留部署记录并重试恢复：{}",
            record.display()
        )),
    }
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
    /// 平台启动 URI（Epic）；空表示直接起 EXE 或 `-applaunch`。
    pub uri: Option<String>,
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
            uri: Option<&'a str>,
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
            uri: self.uri.as_deref(),
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
    use launcher_core::{diagnostic, engine_config, journal, journal_store, resources};

    let mut summary = RunSummary {
        game_id: plan.game_id.clone(),
        proxy: plan.proxy_relative.clone(),
        render: plan.render_executable.clone(),
        engine: plan.engine_path.clone(),
        launch: plan.launch_executable.clone(),
        uri: plan.launch_uri.clone(),
        ..RunSummary::default()
    };
    // 预检放在最前面：一次把缺的东西全列出来。
    let missing = preflight(plan);
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
    let deployment = resources::Deployment::new(&plan.resource_root, &plan.render_directory, proxy);
    let existing = journal_store::load_for_game(launcher_root, &plan.game_id)?;
    if let (true, Some((path, record))) = (options.dry_run, existing.as_ref()) {
        use launcher_core::journal::OperationState;
        record.validate()?;
        summary
            .steps
            .push(format!("restore-required:{}", path.display()));
        for index in record.restore_order_indices() {
            let operation = &record.operations[index];
            if operation.state != OperationState::Restored {
                summary.steps.push(format!(
                    "Recovery: {} {}",
                    operation.kind.as_str(),
                    operation.path,
                ));
            }
        }
        summary.steps.push("dry-run:stop".to_owned());
        return Ok(summary);
    }

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
    let record = journal::Journal::initialize(
        &plan.game_id,
        &session,
        std::process::id(),
        journal_store::format_script_started(now),
        &plan.render_directory,
        &plan.engine_path,
        &plan.engine_runtime_path,
        &plan.backup_path,
        false,
        None,
        false,
        None,
        &plan.proxy_relative,
    );
    // dry-run 不写记录：只按计划模拟每一步。
    if options.dry_run {
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
    // 先恢复上一轮，再按新配置记录磁盘初始状态。
    if existing.is_some() {
        restore_existing(launcher_root, &plan.game_id, false).map_err(|error| {
            Error::invalid(format!(
                "发现上一轮的遗留部署，先恢复它时失败：{error}\n\
                 请先点该行的「恢复原状」处理完再来启动。"
            ))
        })?;
    }

    let mut state = journal_store::DeploymentSession::begin(
        &mutex,
        launcher_root,
        &plan.render_directory,
        record,
        |record| {
            resources::check_proxy_link(&deployment, &ops)?;
            if !plan.engine_path.as_os_str().is_empty() {
                record.original_hash = launcher_core::path::file_hash_or_none(&plan.engine_path)?;
                record.original_existed = record.original_hash.is_some();
            }
            record.storage_existed = deployment.storage_dll().exists();
            Ok(())
        },
    )?;
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
    // 目录在 Steam 库里却按 Direct 启动：这类游戏（Steamworks）被直接启动时常常让 Steam
    // 重启自己，部署会在真正在跑的进程出现之前就被恢复——日志上"部署完成"，游戏却没注入。
    if !plan.launch_mode.eq_ignore_ascii_case("Steam") {
        if let Some(install) = launcher_core::steam::install_of_directory(&plan.game_directory) {
            note(
                &mut log,
                &format!(
                    "注意：这个目录在 Steam 库里（AppID {}，清单 {}）。Steam 游戏被直接启动时\
                     常常会要求 Steam 重启自己，注入窗口会错过；请把这条删掉重新「添加游戏」\
                     一次（会自动识别成 Steam 启动），不要用直接 EXE 启动它。",
                    install.app_id,
                    install.manifest.display()
                ),
            );
        }
    }

    // 二游：先拉起官方启动器，等它出现后再部署（部署完成即注入生效）。
    // Steam 不走这条：它的启动顺序是"先部署 → 再 steam -applaunch → 等游戏"。
    if plan.launcher_process_name.is_some()
        && !plan.launch_mode.eq_ignore_ascii_case("Steam")
        && !options.dry_run
        && !options.skip_launch
    {
        let child = match launcher_win::spawn_process(
            &plan.launch_executable,
            &plan.launch_arguments,
            Some(&plan.game_directory),
            false,
        ) {
            Ok(child) => child,
            Err(error) => return Err(rollback_error(state, &deployment, "启动官方启动器", error)),
        };
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
    let mut deployment_stage = "部署代理链接";
    let deployed: Result<()> = (|| {
        // 1) 代理链接
        resources::install_proxy_link(&mut state, &ops, &deployment, options.dry_run)?;
        summary.steps.push(format!("proxy:{}", plan.proxy_relative));

        // 2) 资源目录链接
        deployment_stage = "部署资源目录";
        let installed =
            resources::install_resources(&mut state, &ops, &deployment, options.dry_run)?;
        summary.resources = installed.created.len();
        summary
            .steps
            .push(format!("resources:{}", installed.created.len()));

        // 3) 诊断 INI（`dlssg_sm86.ini` 链接到 %TEMP% 副本）
        deployment_stage = "部署诊断 INI";
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
        deployment_stage = "部署 Engine.ini";
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
        return Err(rollback_error(state, &deployment, deployment_stage, error));
    }

    // 5) 启动 + 会话等待
    // 二游：官方启动器已经在部署前拉起（见上面的 "launcher:" 步骤），这里只等会话。
    if options.skip_launch {
        summary.steps.push("launch:skipped".to_owned());
    } else {
        // Steam/Epic 都要**先部署再让平台拉起**（游戏进程一启动就加载代理 DLL，部署晚了
        // 注入就错过窗口）；二游走的是"先起官方启动器再部署"那条路。
        // 启动失败必须把部署撤掉：否则游戏没起来、目录里却留着注入文件。
        let launched: Result<()> = (|| {
            if let Some(uri) = plan.launch_uri.as_deref() {
                launcher_win::open_target(uri)?;
                summary.launched = true;
                summary.steps.push("launch:epic".to_owned());
            } else if plan.launcher_process_name.is_none()
                || plan.launch_mode.eq_ignore_ascii_case("Steam")
            {
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
            Ok(())
        })();
        if let Err(error) = launched {
            return Err(rollback_error(state, &deployment, "启动游戏", error));
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
        )
        .map_err(|error| {
            Error::invalid(format!(
                "会话监控失败：{error}\n无法确认游戏已退出，部署记录仍在：{}",
                state.path().display(),
            ))
        })?;
        note(&mut log, &format!("会话结束：{outcome:?}"));
        summary.steps.push(format!("session:{outcome:?}"));
    }

    // 6) 恢复 + 残留检查
    let outcome = restore_deployment(state, &deployment, false)?;
    summary.restored = outcome.is_complete();
    summary
        .steps
        .push(format!("restore:{}", outcome.restored.len()));
    summary.steps.push("verify:clean".to_owned());
    note(&mut log, "恢复完成，残留检查通过");
    if let Some(log) = log.as_ref() {
        summary.steps.push(format!("log:{}", log.path().display()));
    }
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
        // 二游走官方启动器，是 Direct；`GameProfile::new` 的缺省是 Steam（旧档案兼容），
        // 测试里必须写清楚，否则会撞上"Steam 档案缺 AppID"的拒绝。
        profile.launch_mode = "Direct".to_owned();
        profile.config_dir = Some(
            game.join(r"Project\Saved\Config\Windows")
                .to_string_lossy()
                .into_owned(),
        );
        profile.render_executable =
            Some(r"Project\Binaries\Win64\Game-Win64-Shipping.exe".to_owned());
        profile
    }

    fn recovery_record(plan: &RunPlan) -> launcher_core::journal::Journal {
        launcher_core::journal::Journal::initialize(
            &plan.game_id,
            "0123456789abcdef0123456789abcdef",
            0,
            String::new(),
            &plan.render_directory,
            &plan.engine_path,
            &plan.engine_runtime_path,
            &plan.backup_path,
            false,
            None,
            false,
            None,
            &plan.proxy_relative,
        )
    }

    #[test]
    fn restore_finishes_at_the_legacy_record_path() {
        use launcher_core::journal_store;

        let (_sandbox, game, root, paths) = fixture("recover-legacy");
        let initial = plan(&profile(&game), &paths).unwrap();
        std::fs::create_dir(root.join(".state")).unwrap();
        let legacy_record = journal_store::legacy_state_root(&root)
            .join(journal_store::state_relative_path(&initial.game_id));
        journal_store::save_atomic(&legacy_record, &recovery_record(&initial)).unwrap();

        let current = plan(&profile(&game), &paths).unwrap();
        assert!(current.adopted);
        assert_eq!(current.journal_path, legacy_record);
        let saved = std::fs::read(&legacy_record).unwrap();
        let preview = execute(
            &current,
            &root,
            &RunOptions {
                dry_run: true,
                ..RunOptions::default()
            },
        )
        .expect("预览旧根记录");
        assert!(
            preview
                .steps
                .iter()
                .any(|step| step.starts_with("restore-required:"))
        );
        assert_eq!(std::fs::read(&legacy_record).unwrap(), saved);
        restore_existing(&root, &initial.game_id, false).expect("恢复旧根");
        assert!(!legacy_record.exists());
        assert!(!initial.journal_path.exists());
    }

    #[test]
    fn residual_conflict_keeps_record_and_allows_retry() {
        use launcher_core::journal_store;

        let (_sandbox, game, root, paths) = fixture("recover-residue");
        let plan = plan(&profile(&game), &paths).unwrap();
        journal_store::save_atomic(&plan.journal_path, &recovery_record(&plan)).unwrap();
        let proxy = plan.render_directory.join("version.dll");
        std::fs::write(&proxy, b"third-party").unwrap();

        let error = restore_existing(&root, &plan.game_id, false).expect_err("存在残留");
        assert!(error.to_string().contains("residue preserved"), "{error}");
        assert!(plan.journal_path.is_file());
        assert_eq!(std::fs::read(&proxy).unwrap(), b"third-party");

        std::fs::remove_file(&proxy).unwrap();
        restore_existing(&root, &plan.game_id, false).expect("处理冲突后可重试");
        assert!(!plan.journal_path.exists());
    }

    #[test]
    fn rollback_reports_original_failure_and_verification_result() {
        use launcher_core::journal_store::DeploymentSession;

        let (_sandbox, game, root, paths) = fixture("rollback-report");
        let plan = plan(&profile(&game), &paths).unwrap();
        let deployment =
            Deployment::new(&root, &plan.render_directory, ProxyChoice::default_choice());
        let original = || Error::invalid("测试启动失败");
        let open = || {
            DeploymentSession::open(
                &launcher_win::WinMutexOps,
                &root,
                &plan.render_directory,
                recovery_record(&plan),
            )
            .unwrap()
        };

        let rolled_back = rollback_error(open(), &deployment, "启动游戏", original());
        assert!(rolled_back.to_string().contains("测试启动失败"));
        assert!(rolled_back.to_string().contains("残留检查通过"));
        assert!(!plan.journal_path.exists());

        let state = open();
        std::fs::write(plan.render_directory.join("dlssg_sm86.ini"), b"replacement").unwrap();
        let failed = rollback_error(state, &deployment, "启动游戏", original());
        let report = failed.to_string();
        assert!(report.contains("测试启动失败"), "{report}");
        assert!(report.contains("部署回滚失败"), "{report}");
        assert!(report.contains("dlssg_sm86.ini"), "{report}");
        assert!(plan.journal_path.is_file());
    }

    #[test]
    fn payload_moves_into_bin_when_that_subdirectory_exists() {
        let (_sandbox, game, root, _paths) = fixture("bin-layout");
        // 没有 bin：负载根就是启动器根（旧布局）
        assert_eq!(resource_root_of(&root), root);
        // 有 bin：负载根切到 bin
        std::fs::create_dir_all(root.join("bin")).expect("bin");
        assert_eq!(resource_root_of(&root), root.join("bin"));
        // 游戏侧配置目录（preflight 会看它）
        std::fs::create_dir_all(game.join(r"Project\Saved\Config\Windows")).expect("config dir");

        // 把负载挪进 bin，plan 应该照样成立（并且用它去找代理）
        std::fs::rename(root.join("version.dll"), root.join("bin/version.dll")).expect("挪代理");
        std::fs::rename(root.join("dlssg_sm86.ini"), root.join("bin/dlssg_sm86.ini"))
            .expect("挪 ini");
        write(&root, "bin/dlssg_sm86/runtime/nvngx_dlssg.dll", b"stub");
        write(&root, "bin/dlssg_sm86/backend/sm86_backend.dll", b"stub");
        write(&root, "bin/dlssg_sm86/kernels/fg_kernels.dll", b"stub");
        write(&root, "bin/dlssg_sm86/xefg/xefg_kernels.dll", b"stub");
        let paths = PlatformPaths {
            launcher_root: root.clone(),
            temp_directory: std::env::temp_dir().join("dlssg-bin-layout-temp"),
            pid: 4242,
        };
        std::fs::create_dir_all(&paths.temp_directory).expect("temp");
        let run_plan = plan(&profile(&game), &paths).expect("bin 布局应能规划");
        assert_eq!(run_plan.resource_root, root.join("bin"));
        assert!(
            preflight(&run_plan).is_empty(),
            "{:?}",
            preflight(&run_plan)
        );

        // 代理不在 bin 里时应报错并指出 bin
        std::fs::remove_file(root.join("bin/version.dll")).expect("删代理");
        let error = plan(&profile(&game), &paths).expect_err("缺代理应拒绝");
        assert!(error.to_string().contains("bin"), "{error}");
    }

    #[test]
    fn steam_profiles_launch_through_steam_with_derived_arguments() {
        let (sandbox, game, _root, paths) = fixture("steam-plan");
        write(&sandbox.0, "Steam.exe", b"MZ");
        let executable = sandbox.0.join("Steam.exe");
        let mut steam = profile(&game);
        steam.launch_mode = "Steam".to_owned();
        steam.app_id = Some("4115450".to_owned());
        steam.launch_executable = Some(executable.display().to_string());
        steam.launcher_process_name = Some("steam.exe".to_owned());

        let planned = plan(&steam, &paths).expect("Steam 档案应能规划");
        assert_eq!(planned.launch_executable, executable);
        assert_eq!(
            planned.launch_arguments,
            vec!["-applaunch".to_owned(), "4115450".to_owned()],
            "Steam 的启动参数只能由 AppID 得出"
        );
        assert_eq!(planned.launcher_process_name.as_deref(), Some("steam.exe"));
    }

    #[test]
    fn steam_profiles_without_an_app_id_are_refused() {
        let (sandbox, game, _root, paths) = fixture("steam-no-appid");
        write(&sandbox.0, "Steam.exe", b"MZ");
        let executable = sandbox.0.join("Steam.exe");
        let mut steam = profile(&game);
        steam.launch_mode = "Steam".to_owned();
        steam.launch_executable = Some(executable.display().to_string());

        let error = plan(&steam, &paths).expect_err("缺 AppID 必须拒绝，不能直接起 EXE");
        assert!(error.to_string().contains("AppID"), "{error}");
    }

    #[test]
    fn epic_profiles_launch_through_the_launcher_uri() {
        let (_sandbox, game, _root, paths) = fixture("epic-plan");
        let mut epic = profile(&game);
        epic.launch_mode = "Epic".to_owned();
        epic.epic_app_name = Some("Sakura".to_owned());

        let planned = plan(&epic, &paths).expect("Epic 档案应能规划");
        assert_eq!(
            planned.launch_uri.as_deref(),
            Some("com.epicgames.launcher://apps/Sakura?action=launch&silent=true")
        );
        assert_eq!(
            planned.launch_executable, planned.render_executable,
            "Epic 没有单独的启动 EXE"
        );
        assert!(
            planned.launcher_process_name.is_none(),
            "不能把 Epic 启动器当成游戏进程"
        );
        assert!(planned.launch_arguments.is_empty());
    }

    #[test]
    fn epic_profiles_without_an_app_name_are_refused() {
        let (_sandbox, game, _root, paths) = fixture("epic-no-name");
        let mut epic = profile(&game);
        epic.launch_mode = "Epic".to_owned();

        let error = plan(&epic, &paths).expect_err("缺 AppName 必须拒绝");
        assert!(error.to_string().contains("AppName"), "{error}");
    }

    #[test]
    fn epic_games_never_treat_the_epic_launcher_as_their_process() {
        let mut epic = GameProfile::new(
            "direct-0123456789abcdef0123456789abcdef",
            "Sakura",
            PathBuf::from(r"C:\Epic\Sakura"),
        );
        epic.launch_mode = "Epic".to_owned();
        epic.epic_app_name = Some("Sakura".to_owned());
        epic.render_executable = Some(r"Binaries\Win64\Sakura-Win64-Shipping.exe".to_owned());
        // 旧档案可能留着启动 EXE：Epic 模式下也不能把它算成游戏进程。
        epic.launch_executable = Some(
            r"C:\Program Files (x86)\Epic Games\Launcher\Portal\Binaries\Win64\EpicGamesLauncher.exe"
                .to_owned(),
        );
        epic.launcher_process_name = Some("EpicGamesLauncher.exe".to_owned());

        let filter = game_processes(&epic);
        assert!(
            filter
                .executables
                .iter()
                .all(|path| !path.to_string_lossy().to_lowercase().contains("epicgames")),
            "不能把 Epic 启动器当作目标：{:?}",
            filter.executables
        );
        assert!(
            filter
                .names
                .iter()
                .all(|name| !name.to_lowercase().contains("epic")),
            "也不能按名字匹配 Epic 启动器：{:?}",
            filter.names
        );
    }

    #[test]
    fn journal_recovers_the_resource_root_it_was_deployed_from() {
        use launcher_core::journal::{Journal, Operation, OperationKind, OperationState};

        let render = PathBuf::from(r"D:\Game\Binaries\Win64");
        let mut journal = Journal::initialize(
            "nte",
            "0123456789abcdef0123456789abcdef",
            4242,
            "2026-11-23T06:00:00.0000000Z".to_owned(),
            &render,
            Path::new(r"C:\game\Engine.ini"),
            Path::new(r"C:\temp\a-engine.ini"),
            Path::new(r"C:\temp\a.bak.ini"),
            true,
            Some("ABCDEF".to_owned()),
            false,
            None,
            r"alternatives\winmm.dll",
        );
        // 记录里的代理链接：落点在游戏目录，目标在负载根（新布局是 bin）。
        let mut operation = Operation::new(
            OperationKind::Link,
            r"D:\Game\Binaries\Win64\winmm.dll",
            OperationState::Done,
        );
        operation.target = r"D:\launcher\bin\alternatives\winmm.dll".to_owned();
        journal.operations.push(operation);
        let winmm = ProxyChoice::from_name("winmm.dll").unwrap();
        assert_eq!(
            journal_resource_root(&journal, winmm),
            Some(PathBuf::from(r"D:\launcher\bin")),
            "两层代理名要退两层"
        );
        journal.proxy_name = Some("winmm.dll".to_owned());
        assert_eq!(
            journal_resource_root(&journal, winmm),
            Some(PathBuf::from(r"D:\launcher\bin")),
            "旧记录只存文件名时仍使用原负载根"
        );

        // 旧布局（version.dll 在根）也要能推对
        journal.proxy_name = Some("version.dll".to_owned());
        journal.operations.clear();
        let mut operation = Operation::new(
            OperationKind::Link,
            r"D:\Game\Binaries\Win64\version.dll",
            OperationState::Done,
        );
        operation.target = r"D:\launcher\version.dll".to_owned();
        journal.operations.push(operation);
        assert_eq!(
            journal_resource_root(&journal, ProxyChoice::default_choice()),
            Some(PathBuf::from(r"D:\launcher"))
        );
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
