//! Phase 1 步 14–15：把状态机接到进程快照与日志文件上。
//!
//! 状态机本身在 `session.rs` 的单元测试里逐条覆盖；这里只验证"轮询 → 观测 → 推进"的接线，
//! 以及日志文件的建立、写入、清理与增量读取。

use std::cell::RefCell;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use launcher_core::Result;
use launcher_core::journal_store::format_script_started;
use launcher_core::log::{ComponentLogCursor, LauncherLog, Level, cleanup_old_logs};
use launcher_core::ops::{Clock, ProcessEntry, ProcessProbe};
use launcher_core::session::{
    RunningSet, SessionMachine, SessionOutcome, SessionPolicy, SessionSpec, run_session,
};

/// 按脚本依次吐出快照，用完后返回最后一帧。
struct ScriptedProbe {
    snapshots: Vec<Vec<ProcessEntry>>,
    index: RefCell<usize>,
}

impl ScriptedProbe {
    fn new(snapshots: Vec<Vec<ProcessEntry>>) -> Self {
        Self {
            snapshots,
            index: RefCell::new(0),
        }
    }
}

impl ProcessProbe for ScriptedProbe {
    fn list(&self) -> Result<Vec<ProcessEntry>> {
        let mut index = self.index.borrow_mut();
        let snapshot = self
            .snapshots
            .get(*index)
            .or_else(|| self.snapshots.last())
            .cloned()
            .unwrap_or_default();
        *index += 1;
        Ok(snapshot)
    }

    fn current(&self) -> Result<ProcessEntry> {
        Err(launcher_core::Error::invalid("测试不需要 current"))
    }

    fn started_at(&self, _pid: u32) -> Result<SystemTime> {
        Err(launcher_core::Error::invalid("测试不需要 started_at"))
    }
}

/// 每次取时间就前进一个轮询间隔。
struct StepClock {
    current: RefCell<SystemTime>,
    step: Duration,
}

impl Clock for StepClock {
    fn now(&self) -> SystemTime {
        let value = *self.current.borrow();
        *self.current.borrow_mut() = value + self.step;
        value
    }
}

fn process(pid: u32, name: &str) -> ProcessEntry {
    ProcessEntry {
        pid,
        name: name.to_owned(),
        executable: None,
        started_at: None,
    }
}

fn no_sleep(_duration: Duration) {}

#[test]
fn hot_restart_session_runs_through_reappearance() {
    let spec = {
        let mut spec = SessionSpec::new(SessionPolicy::HotRestart {
            grace: Duration::from_secs(10),
        })
        .with_game("wuthering")
        .with_launcher("launcher");
        spec.poll_interval = Duration::from_secs(1);
        spec
    };
    // 启动器出现 → 游戏运行 → 退出 → 3 秒后重启 → 再退出 → 宽限到期
    let snapshots = vec![
        vec![process(1, "launcher")],
        vec![process(2, "wuthering")],
        vec![process(2, "wuthering")],
        Vec::new(),
        Vec::new(),
        vec![process(3, "wuthering")],
        vec![process(3, "wuthering")],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    ];
    let probe = ScriptedProbe::new(snapshots);
    let clock = StepClock {
        current: RefCell::new(UNIX_EPOCH),
        step: Duration::from_secs(1),
    };
    let mut machine = SessionMachine::new(spec.clone(), clock.now());
    let outcome = run_session(&mut machine, &spec, &probe, &clock, &no_sleep).expect("运行");

    match outcome {
        SessionOutcome::GameExited { restarts } => assert_eq!(restarts, 1, "应记录一次热重启"),
        other => panic!("应因游戏退出结束，实际 {other:?}"),
    }
    assert!(machine.is_finished());
}

#[test]
fn first_exit_session_finishes_without_grace() {
    let spec = SessionSpec::new(SessionPolicy::FirstExit)
        .with_game("NTE")
        .with_launcher("NTELauncher");
    let snapshots = vec![
        vec![process(1, "NTELauncher")],
        vec![process(2, "NTE")],
        Vec::new(),
        Vec::new(),
    ];
    let probe = ScriptedProbe::new(snapshots);
    let clock = StepClock {
        current: RefCell::new(UNIX_EPOCH),
        step: Duration::from_secs(1),
    };
    let mut machine = SessionMachine::new(spec.clone(), clock.now());
    let outcome = run_session(&mut machine, &spec, &probe, &clock, &no_sleep).expect("运行");
    assert_eq!(outcome, SessionOutcome::GameExited { restarts: 0 });
}

#[test]
fn startup_timeout_is_reported_by_the_runner() {
    // 默认已改为无期限等待；这里显式配置超时来验证 runner 会把它报出来。
    let mut spec = SessionSpec::new(SessionPolicy::WaitAllTargets).with_game("game");
    spec.tuning.startup_timeout = Duration::from_secs(30);
    let probe = ScriptedProbe::new(vec![Vec::new()]);
    let clock = StepClock {
        current: RefCell::new(UNIX_EPOCH),
        step: Duration::from_secs(30),
    };
    let mut machine = SessionMachine::new(spec.clone(), clock.now());
    let outcome = run_session(&mut machine, &spec, &probe, &clock, &no_sleep).expect("运行");
    assert_eq!(outcome, SessionOutcome::StartupTimedOut);
}

#[test]
fn observe_is_pure_and_uses_the_spec_names() {
    let spec = SessionSpec::new(SessionPolicy::FirstExit)
        .with_game("game")
        .with_launcher("l");
    let set =
        launcher_core::session::observe(&[process(1, "game"), process(2, "other")], &spec).unwrap();
    assert_eq!(
        set,
        RunningSet {
            game_count: 1,
            launcher_running: false
        }
    );
}

#[cfg(windows)]
#[test]
fn full_render_path_excludes_updates_and_other_installs() {
    let spec = SessionSpec::new(SessionPolicy::WaitAllTargets)
        .with_game_executable(std::path::Path::new(r"C:\Games\A\Game.exe"))
        .unwrap();
    let mut game = process(1, "game");
    game.executable = Some(PathBuf::from(r"\\?\c:\games\a\GAME.EXE"));
    let mut other = process(2, "game");
    other.executable = Some(PathBuf::from(r"C:\Games\B\Game.exe"));
    let mut updater = process(3, "updater");
    updater.executable = Some(PathBuf::from(r"C:\Games\A\updater.exe"));
    let set = launcher_core::session::observe(&[game, other, updater], &spec).unwrap();
    assert_eq!(set.game_count, 1);
    assert!(launcher_core::session::observe(&[process(4, "game")], &spec).is_err());
}

#[test]
fn cancellation_waits_until_the_game_is_stopped() {
    let spec = SessionSpec::new(SessionPolicy::WaitAllTargets).with_game("game");
    let clock = StepClock {
        current: RefCell::new(UNIX_EPOCH),
        step: Duration::from_secs(1),
    };
    let probe = ScriptedProbe::new(vec![vec![process(1, "game")], vec![]]);
    let mut machine = SessionMachine::new(spec.clone(), clock.now());
    let outcome = launcher_core::session::run_session_until(
        &mut machine,
        &spec,
        &probe,
        &clock,
        &no_sleep,
        &|| Ok(true),
    )
    .unwrap();
    assert_eq!(outcome, SessionOutcome::Cancelled);
    assert_eq!(*probe.index.borrow(), 2);
}

#[test]
fn launcher_log_round_trips_through_the_file_system() {
    let directory = std::env::temp_dir().join(format!("dlssg-step14-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("临时目录");

    let now = UNIX_EPOCH + Duration::from_secs(1_795_355_769);
    let mut log = LauncherLog::open(&directory, "启动鸣潮", now, 1234).expect("打开日志");
    log.write(Level::Error, "部署失败").expect("写");
    log.write(Level::Detail, "恢复完成").expect("写");
    let path = log.path().to_path_buf();
    assert!(
        path.file_name()
            .expect("文件名")
            .to_string_lossy()
            .starts_with("启动鸣潮_20261122_135609_000_1234")
    );
    let text = std::fs::read_to_string(&path).expect("读日志");
    assert!(text.contains("E 部署失败"), "{text}");
    assert!(text.contains("D 恢复完成"), "{text}");

    // 清理：刚写的日志不能删
    let kept = cleanup_old_logs(&directory.join("logs"), now, Duration::from_secs(24 * 3600))
        .expect("清理");
    assert_eq!(kept, 0);
    assert!(path.exists());

    // 组件日志游标：追加两行后只读到新增
    let component = directory.join("dlssg_sm86.log");
    std::fs::write(
        &component,
        "2026-11-22 13:56:09.523 42 game.exe I {\"a\":1}\r\n",
    )
    .expect("组件日志");
    let mut cursor = ComponentLogCursor::new(&component);
    assert_eq!(cursor.read_new().expect("首读").len(), 1);
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&component)
        .expect("追加");
    use std::io::Write;
    file.write_all(b"2026-11-22 13:56:10.000 42 game.exe D {\"b\":2}\r\n")
        .expect("写");
    drop(file);
    let fresh = cursor.read_new().expect("增量");
    assert_eq!(fresh.len(), 1);
    assert!(fresh[0].contains("\"b\":2"));
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn script_started_helper_is_reused_by_logs() {
    // 两条时间线用同一个格式化函数，避免日志与 journal 出现两套时间写法
    let now = UNIX_EPOCH + Duration::from_secs(1_795_355_769);
    assert_eq!(format_script_started(now), "2026-11-22T13:56:09.0000000Z");
    assert_eq!(LauncherLog::stamp(now), "2026-11-22 13:56:09.000");
    let _ = PathBuf::from("x");
}
