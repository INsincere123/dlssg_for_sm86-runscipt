//! 会话状态机（Phase 1 步 14）。
//!
//! 对应现有实现：
//! - `Wait-ForGameProcess`（`LauncherSession.ps1:15-60`）：轮询游戏进程，记录启动器是否出现过；
//!   启动器出现后又连续 `closed_grace`（默认 3 秒）看不到启动器和游戏 → 放弃启动；
//!   超过 `startup_timeout`（默认 120 秒）没看到游戏 → 放弃。
//! - 三条**不可统一**的退出策略（PARITY C9）：
//!   鸣潮 = 热重启窗口（默认 10 秒）；异环 = 首次退出；Steam/Direct = 等全部目标进程退出。
//!
//! 状态机是纯函数式的：调用方提供"当前有哪些进程在跑"，它给出下一步或最终结果。
//! 这样测试不需要真的等 10 分钟，也不会碰真实进程。

use std::time::{Duration, SystemTime};

use crate::error::{Error, Result};
use crate::ops::{ProcessEntry, ProcessProbe};

/// 退出策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPolicy {
    /// 游戏退出后继续等 `grace`；期间重新出现就继续等下一次退出。
    HotRestart { grace: Duration },
    /// 首次退出即结束。
    FirstExit,
    /// 等全部目标进程都退出（按名字集合判定）。
    WaitAllTargets,
}

impl SessionPolicy {
    /// 鸣潮的默认热重启窗口。
    pub fn wuthering() -> Self {
        SessionPolicy::HotRestart {
            grace: Duration::from_secs(10),
        }
    }

    pub fn neverness() -> Self {
        SessionPolicy::FirstExit
    }

    pub fn steam_or_direct() -> Self {
        SessionPolicy::WaitAllTargets
    }

    pub fn label(self) -> &'static str {
        match self {
            SessionPolicy::HotRestart { .. } => "HotRestart",
            SessionPolicy::FirstExit => "FirstExit",
            SessionPolicy::WaitAllTargets => "WaitAllTargets",
        }
    }
}

/// 启动阶段等待的参数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartupTuning {
    /// 等游戏出现的最长时间。
    pub startup_timeout: Duration,
    /// 启动器消失后确认"确实关了"的宽限。
    pub closed_grace: Duration,
}

impl Default for StartupTuning {
    fn default() -> Self {
        Self {
            startup_timeout: Duration::ZERO,
            closed_grace: Duration::from_secs(3),
        }
    }
}

/// 会话配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSpec {
    /// 参与判定的游戏进程名（不含 `.exe`，大小写不敏感）。
    pub game_processes: Vec<String>,
    /// 启动器进程名；游戏出现前用它们判断"启动器是否还开着"。
    pub launcher_processes: Vec<String>,
    /// 启动器的完整 EXE 路径（可选）。给了就要求进程路径一致才认（N-2）。
    pub launcher_executable: Option<String>,
    pub policy: SessionPolicy,
    pub tuning: StartupTuning,
    pub poll_interval: Duration,
}

impl SessionSpec {
    pub fn new(policy: SessionPolicy) -> Self {
        Self {
            game_processes: Vec::new(),
            launcher_processes: Vec::new(),
            launcher_executable: None,
            policy,
            tuning: StartupTuning::default(),
            poll_interval: Duration::from_secs(1),
        }
    }

    pub fn with_game(mut self, name: impl Into<String>) -> Self {
        self.game_processes.push(name.into());
        self
    }

    pub fn with_launcher(mut self, name: impl Into<String>) -> Self {
        self.launcher_processes.push(name.into());
        self
    }

    pub fn with_launcher_executable(mut self, path: impl Into<String>) -> Self {
        self.launcher_executable = Some(path.into());
        self
    }
}

/// 每轮观测到的运行集合。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunningSet {
    /// 目标进程数量（可能匹配多个同名进程）。
    pub game_count: usize,
    pub launcher_running: bool,
}

/// 会话结束原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionOutcome {
    /// 游戏正常退出（`restarts` 是热重启窗口里重新出现的次数）。
    GameExited { restarts: u32 },
    /// 启动器先关了、游戏一直没出现：按现有实现放弃启动并走恢复。
    LauncherClosedBeforeGame,
    /// 超过启动超时。
    StartupTimedOut,
}

/// 状态机内部状态。
#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    WaitingForGame {
        launcher_observed: bool,
        closed_since: Option<SystemTime>,
    },
    Running,
    RestartGrace {
        since: SystemTime,
    },
}

/// 会话状态机。
#[derive(Debug, Clone)]
pub struct SessionMachine {
    spec: SessionSpec,
    state: State,
    started_at: SystemTime,
    restarts: u32,
    finished: Option<SessionOutcome>,
}

impl SessionMachine {
    pub fn new(spec: SessionSpec, now: SystemTime) -> Self {
        Self {
            spec,
            state: State::WaitingForGame {
                launcher_observed: false,
                closed_since: None,
            },
            started_at: now,
            restarts: 0,
            finished: None,
        }
    }

    pub fn outcome(&self) -> Option<&SessionOutcome> {
        self.finished.as_ref()
    }

    pub fn is_finished(&self) -> bool {
        self.finished.is_some()
    }

    pub fn restarts(&self) -> u32 {
        self.restarts
    }

    /// 喂一轮观测结果。
    pub fn update(&mut self, running: RunningSet, now: SystemTime) -> Option<SessionOutcome> {
        if let Some(outcome) = &self.finished {
            return Some(outcome.clone());
        }
        let outcome = self.step(running, now);
        if let Some(outcome) = &outcome {
            self.finished = Some(outcome.clone());
        }
        outcome
    }

    fn step(&mut self, running: RunningSet, now: SystemTime) -> Option<SessionOutcome> {
        match self.state.clone() {
            State::WaitingForGame {
                mut launcher_observed,
                mut closed_since,
            } => {
                if running.game_count > 0 {
                    self.state = State::Running;
                    return None;
                }
                if running.launcher_running {
                    launcher_observed = true;
                    closed_since = None;
                } else if launcher_observed {
                    // 启动器先关、游戏还没出现：**不结束会话**，继续等游戏。
                    // 二游的官方启动器把游戏拉起后自己退出是常见行为，
                    // 只有"启动器关了且游戏也退出"才算会话结束。
                    let _ = closed_since.get_or_insert(now);
                }
                // 无期限等待：常驻程序不会因为"游戏暂时没出现"就放弃会话。
                if !self.spec.tuning.startup_timeout.is_zero()
                    && now.duration_since(self.started_at).unwrap_or_default()
                        >= self.spec.tuning.startup_timeout
                {
                    return Some(SessionOutcome::StartupTimedOut);
                }
                self.state = State::WaitingForGame {
                    launcher_observed,
                    closed_since,
                };
                None
            }
            State::Running => {
                if running.game_count > 0 {
                    return None;
                }
                match self.spec.policy {
                    SessionPolicy::FirstExit => Some(SessionOutcome::GameExited {
                        restarts: self.restarts,
                    }),
                    SessionPolicy::WaitAllTargets => Some(SessionOutcome::GameExited {
                        restarts: self.restarts,
                    }),
                    SessionPolicy::HotRestart { .. } => {
                        self.state = State::RestartGrace { since: now };
                        None
                    }
                }
            }
            State::RestartGrace { since } => {
                if running.game_count > 0 {
                    self.restarts += 1;
                    self.state = State::Running;
                    return None;
                }
                let SessionPolicy::HotRestart { grace } = self.spec.policy else {
                    // 只有热重启策略会进入这个状态；其他策略直接结束。
                    return Some(SessionOutcome::GameExited {
                        restarts: self.restarts,
                    });
                };
                if now.duration_since(since).unwrap_or_default() >= grace {
                    return Some(SessionOutcome::GameExited {
                        restarts: self.restarts,
                    });
                }
                None
            }
        }
    }
}

/// 把状态机接到真实的进程快照上。
pub fn run_session(
    machine: &mut SessionMachine,
    spec: &SessionSpec,
    probe: &dyn ProcessProbe,
    clock: &dyn crate::ops::Clock,
    sleep: &dyn Fn(Duration),
) -> Result<SessionOutcome> {
    loop {
        let processes = probe.list()?;
        let running = observe(&processes, spec);
        let now = clock.now();
        if let Some(outcome) = machine.update(running, now) {
            return Ok(outcome);
        }
        if spec.poll_interval.is_zero() {
            return Err(Error::invalid("轮询间隔不能为 0。"));
        }
        sleep(spec.poll_interval);
    }
}

/// 按配置把进程列表折算成运行集合。
pub fn observe(processes: &[ProcessEntry], spec: &SessionSpec) -> RunningSet {
    let game_count = processes
        .iter()
        .filter(|entry| matches_name(&spec.game_processes, &entry.name))
        .count();
    let launcher_running = processes
        .iter()
        .any(|entry| matches_name(&spec.launcher_processes, &entry.name));
    RunningSet {
        game_count,
        launcher_running,
    }
}

/// 官方启动器是否已经在运行（N-2：复用已运行的启动器，不再拉起）。
///
/// 给了 `launcher_executable` 时按**完整路径**核对；路径拿不到（权限不足）时返回 `false`，
/// 不做猜测——宁可当作没运行，也不能因为同名进程误判成"已运行"而拒绝启动。
pub fn launcher_already_running(probe: &dyn ProcessProbe, spec: &SessionSpec) -> Result<bool> {
    if spec.launcher_processes.is_empty() {
        return Ok(false);
    }
    let processes = probe.list()?;
    let context = match &spec.launcher_executable {
        Some(_) => Some(crate::path::PathContext::current()?),
        None => None,
    };
    let wanted_path = match (&spec.launcher_executable, &context) {
        (Some(path), Some(context)) => Some(context.normalize(path)?),
        _ => None,
    };
    for entry in &processes {
        if !matches_name(&spec.launcher_processes, &entry.name) {
            continue;
        }
        match (&wanted_path, &entry.executable) {
            (Some(wanted), Some(actual)) => {
                let actual = context
                    .as_ref()
                    .expect("有 wanted_path 就一定有上下文")
                    .normalize(&actual.to_string_lossy())?;
                if &actual == wanted {
                    return Ok(true);
                }
            }
            (Some(_), None) => {}
            (None, _) => return Ok(true),
        }
    }
    Ok(false)
}

fn matches_name(names: &[String], candidate: &str) -> bool {
    names
        .iter()
        .any(|name| name.eq_ignore_ascii_case(candidate))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    fn at(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
    }

    fn running(game: usize, launcher: bool) -> RunningSet {
        RunningSet {
            game_count: game,
            launcher_running: launcher,
        }
    }

    #[test]
    fn default_tuning_matches_the_existing_implementation() {
        let tuning = StartupTuning::default();
        // 默认无期限等待：GUI 常驻时不会因为"游戏暂时没出现"结束会话。
        assert_eq!(tuning.startup_timeout, Duration::ZERO);
        assert_eq!(tuning.closed_grace, Duration::from_secs(3));
        assert_eq!(SessionPolicy::wuthering().label(), "HotRestart");
        assert_eq!(SessionPolicy::neverness().label(), "FirstExit");
        assert_eq!(SessionPolicy::steam_or_direct().label(), "WaitAllTargets");
    }

    #[test]
    fn first_exit_finishes_immediately() {
        let mut machine = SessionMachine::new(SessionSpec::new(SessionPolicy::FirstExit), at(0));
        assert!(machine.update(running(1, true), at(1)).is_none());
        // FirstExit 在观测到退出当轮就结束，不做额外宽限
        assert_eq!(
            machine.update(running(0, true), at(2)),
            Some(SessionOutcome::GameExited { restarts: 0 })
        );
    }

    #[test]
    fn hot_restart_keeps_waiting_inside_the_window() {
        let mut machine = SessionMachine::new(
            SessionSpec::new(SessionPolicy::HotRestart {
                grace: Duration::from_secs(10),
            }),
            at(0),
        );
        machine.update(running(1, false), at(1));
        // 第一次退出 → 进入宽限
        assert!(machine.update(running(0, false), at(5)).is_none());
        // 宽限内重启 → 回到运行态
        assert!(machine.update(running(1, false), at(8)).is_none());
        assert_eq!(machine.restarts(), 1);
        // 再次退出 → 重新计时
        assert!(machine.update(running(0, false), at(20)).is_none());
        // 宽限还没到
        assert!(machine.update(running(0, false), at(29)).is_none());
        // 到点结束，并记录重启次数
        assert_eq!(
            machine.update(running(0, false), at(30)),
            Some(SessionOutcome::GameExited { restarts: 1 })
        );
        assert!(machine.is_finished());
    }

    #[test]
    fn launcher_closed_before_game_keeps_waiting() {
        // 启动器把游戏拉起后自己退出是常见行为：不能因此结束会话。
        let mut machine =
            SessionMachine::new(SessionSpec::new(SessionPolicy::WaitAllTargets), at(0));
        assert!(machine.update(running(0, true), at(1)).is_none());
        assert!(machine.update(running(0, false), at(2)).is_none());
        assert!(machine.update(running(0, false), at(4)).is_none());
        assert!(machine.update(running(0, false), at(5)).is_none());
        // 游戏之后出现并退出，才算结束
        assert!(machine.update(running(1, false), at(6)).is_none());
        assert!(machine.update(running(0, false), at(8)).is_some());
    }

    #[test]
    fn startup_timeout_only_applies_when_configured() {
        // 默认无期限：从未见过游戏也一直等（常驻程序语义）
        let mut forever = SessionMachine::new(SessionSpec::new(SessionPolicy::FirstExit), at(0));
        assert!(forever.update(running(0, false), at(600)).is_none());

        // 显式配置超时后仍然生效
        let mut spec = SessionSpec::new(SessionPolicy::FirstExit);
        spec.tuning.startup_timeout = Duration::from_secs(30);
        let mut machine = SessionMachine::new(spec, at(0));
        assert!(machine.update(running(0, false), at(10)).is_none());
        assert_eq!(
            machine.update(running(0, false), at(40)),
            Some(SessionOutcome::StartupTimedOut)
        );
    }

    #[test]
    fn game_appearing_after_launcher_observes_normally() {
        let mut machine = SessionMachine::new(SessionSpec::new(SessionPolicy::FirstExit), at(0));
        machine.update(running(0, true), at(1));
        assert!(machine.update(running(2, true), at(2)).is_none());
        assert_eq!(
            machine.update(running(0, true), at(3)),
            Some(SessionOutcome::GameExited { restarts: 0 })
        );
    }

    #[test]
    fn observe_matches_names_case_insensitively() {
        let spec = SessionSpec::new(SessionPolicy::FirstExit)
            .with_game("b1-Win64-Shipping")
            .with_launcher("NTELauncher");
        let processes = vec![
            ProcessEntry {
                pid: 1,
                name: "B1-WIN64-SHIPPING".to_owned(),
                executable: None,
                started_at: None,
            },
            ProcessEntry {
                pid: 2,
                name: "ntelauncher".to_owned(),
                executable: None,
                started_at: None,
            },
            ProcessEntry {
                pid: 3,
                name: "explorer".to_owned(),
                executable: None,
                started_at: None,
            },
        ];
        let set = observe(&processes, &spec);
        assert_eq!(set.game_count, 1);
        assert!(set.launcher_running);
    }

    #[test]
    fn launcher_already_running_uses_path_when_available() {
        let spec = SessionSpec::new(SessionPolicy::FirstExit)
            .with_game("game")
            .with_launcher("launcher")
            .with_launcher_executable(r"D:\Launcher\launcher.exe");
        let matching = crate::testing::FakeProcessProbe::empty().with_entry(ProcessEntry {
            pid: 10,
            name: "launcher".to_owned(),
            executable: Some(PathBuf::from(r"d:\launcher\LAUNCHER.exe")),
            started_at: None,
        });
        assert!(launcher_already_running(&matching, &spec).expect("查询"));

        let different = crate::testing::FakeProcessProbe::empty().with_entry(ProcessEntry {
            pid: 11,
            name: "launcher".to_owned(),
            executable: Some(PathBuf::from(r"E:\Other\launcher.exe")),
            started_at: None,
        });
        assert!(!launcher_already_running(&different, &spec).expect("查询"));

        // 路径拿不到时不能猜"已运行"
        let unknown_path = crate::testing::FakeProcessProbe::empty().with_entry(ProcessEntry {
            pid: 12,
            name: "launcher".to_owned(),
            executable: None,
            started_at: None,
        });
        assert!(!launcher_already_running(&unknown_path, &spec).expect("查询"));
    }

    #[test]
    fn launcher_already_running_falls_back_to_name_without_path_rule() {
        let spec = SessionSpec::new(SessionPolicy::FirstExit)
            .with_game("game")
            .with_launcher("NTELauncher");
        let probe = crate::testing::FakeProcessProbe::empty().with_entry(ProcessEntry {
            pid: 13,
            name: "ntelauncher".to_owned(),
            executable: None,
            started_at: None,
        });
        assert!(launcher_already_running(&probe, &spec).expect("查询"));

        let empty = SessionSpec::new(SessionPolicy::FirstExit).with_game("game");
        assert!(!launcher_already_running(&probe, &empty).expect("无启动器名时恒为 false"));
    }

    #[test]
    fn zero_poll_interval_is_rejected() {
        struct Empty;
        impl ProcessProbe for Empty {
            fn list(&self) -> Result<Vec<ProcessEntry>> {
                Ok(Vec::new())
            }
            fn current(&self) -> Result<ProcessEntry> {
                Err(Error::invalid("n/a"))
            }
            fn started_at(&self, _pid: u32) -> Result<SystemTime> {
                Err(Error::invalid("n/a"))
            }
        }
        let mut spec = SessionSpec::new(SessionPolicy::FirstExit);
        spec.poll_interval = Duration::ZERO;
        let mut machine = SessionMachine::new(spec.clone(), at(0));
        let probe = Empty;
        let clock = crate::testing::FixedClock::at_epoch_seconds(0);
        let error = run_session(&mut machine, &spec, &probe, &clock, &|_| {}).expect_err("应拒绝");
        assert!(error.to_string().contains("轮询间隔"), "实际：{error}");
    }
}
