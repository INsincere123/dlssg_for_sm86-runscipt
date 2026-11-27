//! `DlssgLauncher` 命令行契约（M2/M4/M8）：无控制台、参数校验、退出码、单会话、引擎编排。

use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

// 单会话是机器级互斥：涉及会话锁的用例必须串行。
static SESSION_LOCK: Mutex<()> = Mutex::new(());

/// 等机器级的单会话锁空出来（最多 30 秒）。
///
/// 这把锁不只测试在用：用户开着的启动器窗口、真在跑的游戏会话都会占着它
/// （`Global\DlssgSession`）。占着时引擎会按设计返回 `session-busy`，
/// 用例若直接开跑就会把"别人正在跑游戏"误判成失败（真机 2026-11-23 遇到过）。
fn wait_for_free_session() {
    let deadline = Instant::now() + Duration::from_secs(30);
    while launcher_win::session_active() {
        assert!(
            Instant::now() < deadline,
            "等待单会话锁超时：本机还有引擎或游戏会话在跑，稍后重试"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_DlssgLauncher"))
}

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("dlssg-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("临时目录");
    directory
}

fn run(arguments: &[&str]) -> std::process::ExitStatus {
    Command::new(binary())
        .args(arguments)
        .status()
        .expect("启动 launcher")
}

fn write_file(root: &std::path::Path, relative: &str) {
    let path = root.join(relative.replace('/', "\\"));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("父目录");
    }
    std::fs::write(&path, b"stub").expect("写文件");
}

/// 可规划、可部署的最小环境：资源、配置源、假游戏（含原件）、偏好文件。
fn engine_fixture(name: &str) -> (PathBuf, PathBuf) {
    let root = scratch(name);
    for relative in [
        "配置/fixture/Engine.ini",
        "dlssg_sm86.ini",
        "version.dll",
        "dlssg_sm86/runtime/nvngx_dlssg.dll",
        "dlssg_sm86/backend/sm86_backend.dll",
        "dlssg_sm86/kernels/fg_kernels.dll",
        "dlssg_sm86/xefg/xefg_kernels.dll",
        "dlssg_sm86/sr/sr_kernels.dll",
        "dlssg_sm86/sr/nvngx_dlss.dll",
        "dlssg_sm86/version.dll",
    ] {
        write_file(&root, relative);
    }
    let game = root.join("game");
    std::fs::create_dir_all(game.join("Project/Binaries/Win64")).expect("游戏目录");
    std::fs::write(
        game.join("Project/Binaries/Win64/Game-Win64-Shipping.exe"),
        b"MZ",
    )
    .expect("渲染程序");
    let engine = game.join("Project/Saved/Config/Windows/Engine.ini");
    std::fs::create_dir_all(engine.parent().expect("父目录")).expect("配置目录");
    std::fs::write(&engine, b"original=\r\n").expect("游戏原件");

    let preferences = root.join("preferences.json");
    let text = format!(
        "{{\"version\":1,\"games\":[{{\"id\":\"fixture\",\"name\":\"夹具\",\"directory\":\"{0}\",\"config_dir\":\"{0}/Project/Saved/Config/Windows\",\"launch_mode\":\"Direct\",\"engine_host\":\"UE5\",\"quality_tier\":\"Tier2\",\"proxy_index\":1,\"render_executable\":\"Project\\\\Binaries\\\\Win64\\\\Game-Win64-Shipping.exe\"}}]}}",
        game.to_string_lossy().replace('\\', "\\\\")
    );
    std::fs::write(&preferences, text).expect("偏好");
    (root, preferences)
}

fn process_fixture_journal(root: &std::path::Path, render: PathBuf, session: &str) -> PathBuf {
    use launcher_core::journal::{DeploymentProcesses, Journal};
    use launcher_core::journal_store;

    let path = root.join(journal_store::state_relative_path("fixture"));
    let mut journal =
        Journal::from_json(&launcher_core::fixtures::read_text("journal/01-new.json")).unwrap();
    journal.session = session.to_owned();
    journal.render = render.parent().unwrap().display().to_string();
    journal.engine = String::new();
    journal.runtime = String::new();
    journal.backup = String::new();
    journal.processes = Some(DeploymentProcesses {
        render,
        launchers: vec![],
    });
    journal_store::save_atomic(&path, &journal).unwrap();
    path
}

#[test]
fn engine_check_writes_a_report_without_a_console_window() {
    let directory = scratch("check");
    let report = directory.join("report.json");
    let status = run(&[
        "--engine",
        "check",
        "--report",
        report.to_str().expect("路径"),
    ]);
    assert!(status.success(), "check 应成功，实际 {status:?}");
    let text = std::fs::read_to_string(&report).expect("读报告");
    assert!(text.contains("\"command\": \"check\""), "{text}");
    assert!(
        text.contains("\"console_window\": 0"),
        "windows 子系统不应有控制台窗口：{text}"
    );
    assert!(text.contains("\"elevated\": "), "{text}");
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn manual_restore_checks_render_path_and_stop_only_ends_the_target() {
    let (root, preferences) = engine_fixture("verified-processes");
    let render = root.join("game/Project/Binaries/Win64/Game-Win64-Shipping.exe");
    let other_dir = root.join("other");
    std::fs::create_dir(&other_dir).unwrap();
    let same_name = other_dir.join(render.file_name().unwrap());
    let helper = render.with_file_name("Updater.exe");
    for path in [&render, &same_name, &helper] {
        std::fs::copy(binary(), path).unwrap();
    }
    let mut children: Vec<_> = [&render, &same_name, &helper]
        .into_iter()
        .map(|path| {
            Command::new(path)
                .args(["--engine", "check", "--sleep-ms", "15000"])
                .spawn()
                .unwrap()
        })
        .collect();

    let session = format!("a{:031x}", std::process::id());
    let journal_path = process_fixture_journal(&root, render, &session);
    let before = std::fs::read(&journal_path).unwrap();

    let report = root.join("report.json");
    let invoke = |action| {
        let status = run(&[
            "--engine",
            action,
            "--game",
            "fixture",
            "--root",
            root.to_str().unwrap(),
            "--preferences",
            preferences.to_str().unwrap(),
            "--report",
            report.to_str().unwrap(),
        ]);
        let result =
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(&report).unwrap()).unwrap();
        (status, result)
    };
    let (blocked_status, blocked) = invoke("restore");
    let retained = std::fs::read(&journal_path).unwrap();
    let (stopped_status, stopped) = invoke("stop");
    let exited: Vec<_> = children
        .iter_mut()
        .map(|child| child.try_wait().unwrap().is_some())
        .collect();
    for child in &mut children {
        let _ = child.kill();
        let _ = child.wait();
    }

    assert_eq!(blocked_status.code(), Some(2));
    assert!(
        blocked["error"]
            .as_str()
            .unwrap()
            .contains("目标进程仍在运行"),
        "{blocked}"
    );
    assert_eq!(before, retained);
    assert!(stopped_status.success(), "{stopped}");
    assert_eq!(stopped["killed"], 1);
    assert_eq!(exited, vec![true, false, false]);
    assert!(!journal_path.exists());
}

#[test]
fn stop_signals_a_waiting_session_before_reporting_recovery() {
    use launcher_win::activate::SessionCancellation;

    let (root, preferences) = engine_fixture("cancel-waiting");
    let session = format!("b{:031x}", std::process::id());
    let render = root.join("game/Project/Binaries/Win64/Game-Win64-Shipping.exe");
    let journal = process_fixture_journal(&root, render, &session);
    let cancellation = SessionCancellation::create(&session).unwrap();
    let engine_root = root.clone();
    let receiver = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !cancellation.requested() {
            assert!(Instant::now() < deadline, "等待会话未收到中止信号");
            std::thread::sleep(Duration::from_millis(25));
        }
        run(&[
            "--engine",
            "restore",
            "--game",
            "fixture",
            "--root",
            engine_root.to_str().unwrap(),
        ])
    });
    let report = root.join("stop.json");
    let stopped = run(&[
        "--engine",
        "stop",
        "--game",
        "fixture",
        "--root",
        root.to_str().unwrap(),
        "--preferences",
        preferences.to_str().unwrap(),
        "--report",
        report.to_str().unwrap(),
    ]);
    assert!(receiver.join().unwrap().success());
    assert!(stopped.success());
    let result: serde_json::Value =
        serde_json::from_slice(&std::fs::read(report).unwrap()).unwrap();
    assert_eq!(result["killed"], 0);
    assert_eq!(result["restored"], true);
    assert!(!journal.exists());
}

#[test]
fn ui_restore_carries_the_request_identity_on_success_and_failure() {
    let (root, preferences) = engine_fixture("ui-restore");
    let render = root.join("game/Project/Binaries/Win64/Game-Win64-Shipping.exe");
    let journal = process_fixture_journal(&root, render, "11111111111111111111111111111111");
    for (request, expected) in [
        ("22222222222222222222222222222222", "completed"),
        ("33333333333333333333333333333333", "failed"),
    ] {
        let report = root.join(format!("ui-restore-{request}.json"));
        let child = Command::new(binary())
            .args([
                "--engine",
                "restore",
                "--game",
                "fixture",
                "--ui-request",
                request,
                "--root",
                root.to_str().unwrap(),
                "--preferences",
                preferences.to_str().unwrap(),
                "--report",
                report.to_str().unwrap(),
            ])
            .spawn()
            .unwrap();
        let pid = child.id();
        let status = child.wait_with_output().unwrap().status;
        let result: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
        assert_eq!(status.success(), expected == "completed");
        assert_eq!(result["request"]["id"], request);
        assert_eq!(result["request"]["game"], "fixture");
        assert_eq!(result["request"]["command"], "restore");
        assert_eq!(result["pid"], pid);
        assert_eq!(result["outcome"]["status"], expected);
        assert!(!report.with_extension("accepted.json").exists());
    }
    assert!(!journal.exists());
}

#[test]
fn busy_ui_run_emits_only_a_bound_terminal_result() {
    let _tests = SESSION_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    wait_for_free_session();
    let _session = launcher_win::acquire_single_session(Duration::ZERO)
        .unwrap()
        .expect("预占会话锁");
    let root = scratch("ui-busy");
    let report = root.join("ui-run.json");
    let request = "44444444444444444444444444444444";
    let status = run(&[
        "--engine",
        "run",
        "--game",
        "fixture",
        "--ui-request",
        request,
        "--root",
        root.to_str().unwrap(),
        "--report",
        report.to_str().unwrap(),
    ]);
    let result: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
    assert_eq!(status.code(), Some(6));
    assert_eq!(result["request"]["id"], request);
    assert_eq!(result["outcome"]["status"], "session-busy");
    assert!(!report.with_extension("accepted.json").exists());
    assert!(!root.join(".state").exists());
}

#[test]
fn unknown_arguments_exit_with_usage_code() {
    assert_eq!(run(&["--nope"]).code(), Some(2));
    assert_eq!(run(&["--engine"]).code(), Some(2));
    assert_eq!(run(&["--engine", "bogus"]).code(), Some(2));
    assert_eq!(run(&["--engine", "run"]).code(), Some(2));
}

#[test]
fn restore_requires_a_game_and_reports_missing_records() {
    // 缺 --game 是参数错误
    assert_eq!(run(&["--engine", "restore"]).code(), Some(2));
    // 记录不存在时明确报错，而不是假装恢复成功
    let directory = scratch("restore-none");
    let report = directory.join("r.json");
    let status = run(&[
        "--engine",
        "restore",
        "--game",
        "fixture",
        "--root",
        directory.to_str().expect("路径"),
        "--report",
        report.to_str().expect("路径"),
    ]);
    assert_eq!(status.code(), Some(2));
    let text = std::fs::read_to_string(&report).expect("报告");
    assert!(text.contains("恢复失败"), "{text}");
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn run_without_a_known_game_is_a_usage_error() {
    let directory = scratch("no-game");
    let report = directory.join("r.json");
    let status = run(&[
        "--engine",
        "run",
        "--game",
        "missing",
        "--root",
        directory.to_str().expect("路径"),
        "--report",
        report.to_str().expect("路径"),
    ]);
    assert_eq!(status.code(), Some(2));
    let text = std::fs::read_to_string(&report).expect("错误报告");
    assert!(text.contains("error"), "{text}");
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn version_is_reported_into_the_report_file() {
    let directory = scratch("version");
    let report = directory.join("version.json");
    let status = run(&["--version", "--report", report.to_str().expect("路径")]);
    assert!(status.success());
    let text = std::fs::read_to_string(&report).expect("读报告");
    assert!(text.contains(env!("CARGO_PKG_VERSION")), "{text}");
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn elevate_runs_the_child_and_the_child_reports_high_integrity() {
    let directory = scratch("elevate");
    let report = directory.join("report.json");
    let status = run(&[
        "--engine",
        "check",
        "--elevate",
        "--report",
        report.to_str().expect("路径"),
    ]);
    let code = status.code();
    assert!(
        matches!(code, Some(0) | Some(4) | Some(5)),
        "提权结果应是 0/4/5，实际 {code:?}"
    );
    let text = std::fs::read_to_string(&report).expect("父报告");
    if code == Some(5) {
        assert!(text.contains("\"elevation\": \"cancelled\""), "{text}");
        let _ = std::fs::remove_dir_all(&directory);
        return;
    }
    assert!(text.contains("\"elevation\": \"finished\""), "{text}");
    if code == Some(0) {
        let child = directory.join("elevated-child.json");
        let child_text = std::fs::read_to_string(&child).expect("子报告");
        assert!(
            child_text.contains("\"elevated\": true"),
            "子进程应处于高完整性：{child_text}"
        );
    }
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn dry_run_reports_the_plan_without_touching_anything() {
    let _guard = SESSION_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    wait_for_free_session();
    let (root, preferences) = engine_fixture("plan");
    let report = root.join("plan.json");
    let status = run(&[
        "--engine",
        "run",
        "--game",
        "fixture",
        "--dry-run",
        "--root",
        root.to_str().expect("路径"),
        "--preferences",
        preferences.to_str().expect("路径"),
        "--report",
        report.to_str().expect("路径"),
    ]);
    assert_eq!(status.code(), Some(0), "dry-run 应成功");
    let text = std::fs::read_to_string(&report).expect("执行报告");
    for key in [
        "\"code\": \"executed\"",
        "\"game\": \"fixture\"",
        "\"proxy\": \"version.dll\"",
        "\"diagnostic\": true",
        "\"engine_copy\": true",
        "\"dry-run:stop\"",
    ] {
        assert!(text.contains(key), "dry-run 报告缺少 {key}：{text}");
    }
    // 不落盘：无记录、无链接、无临时副本
    assert!(
        !root.join(".state/fixture/active.json").exists(),
        "不应写记录"
    );
    let render = root.join("game/Project/Binaries/Win64");
    assert!(!render.join("version.dll").exists(), "不应建代理链接");
    assert!(!render.join("dlssg_sm86.ini").exists(), "不应建诊断链接");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn deploy_and_restore_a_fake_game_without_launching() {
    let _guard = SESSION_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    wait_for_free_session();
    let (root, preferences) = engine_fixture("deploy");
    let engine = root.join("game/Project/Saved/Config/Windows/Engine.ini");
    let report = root.join("run.json");
    let status = run(&[
        "--engine",
        "run",
        "--game",
        "fixture",
        "--skip-launch",
        "--root",
        root.to_str().expect("路径"),
        "--preferences",
        preferences.to_str().expect("路径"),
        "--report",
        report.to_str().expect("路径"),
    ]);
    assert_eq!(
        status.code(),
        Some(0),
        "skip-launch 应成功；报告见 {}",
        report.display()
    );
    let text = std::fs::read_to_string(&report).expect("报告");
    assert!(text.contains("\"restored\": true"), "应完成恢复：{text}");
    assert!(text.contains("\"verify:clean\""), "残留检查应通过：{text}");
    assert_eq!(
        std::fs::read(&engine).expect("原件"),
        b"original=\r\n",
        "Engine.ini 必须按字节恢复"
    );
    assert!(
        !root.join(".state/fixture/active.json").exists(),
        "记录应删除"
    );
    let render = root.join("game/Project/Binaries/Win64");
    for leftover in ["version.dll", "dlssg_sm86.ini", "dlssg_sm86"] {
        assert!(!render.join(leftover).exists(), "残留：{leftover}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// 全链路：部署 → 启动一个真的会活一会儿的进程 → 等待退出 → 自动恢复。
#[test]
fn launch_wait_and_restore_a_fake_game() {
    let _guard = SESSION_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    wait_for_free_session();
    let (root, preferences) = engine_fixture("launch");
    // 用自身扮演游戏进程：check 的 --sleep-ms 让它活 1200ms 再退出。
    let self_exe = binary();
    let fake_render = root.join("game/Project/Binaries/Win64/Game-Win64-Shipping.exe");
    std::fs::copy(&self_exe, &fake_render).expect("复制自身作为假游戏");
    // 偏好里给它 1.2 秒的存活时间 + 让会话按"首次退出"判定
    let preferences_text = std::fs::read_to_string(&preferences).expect("偏好");
    let preferences_text = preferences_text.replace(
        "\"id\":\"fixture\"",
        "\"id\":\"nte\",\"launch_arguments\":[\"--engine\",\"check\",\"--sleep-ms\",\"1200\"]",
    );
    std::fs::write(&preferences, preferences_text).expect("改偏好");
    std::fs::create_dir_all(root.join("配置/nte")).expect("配置源目录");
    std::fs::copy(
        root.join("配置/fixture/Engine.ini"),
        root.join("配置/nte/Engine.ini"),
    )
    .expect("配置源");

    let report = root.join("launch.json");
    let status = run(&[
        "--engine",
        "run",
        "--game",
        "nte",
        "--root",
        root.to_str().expect("路径"),
        "--preferences",
        preferences.to_str().expect("路径"),
        "--report",
        report.to_str().expect("路径"),
    ]);
    assert_eq!(
        status.code(),
        Some(0),
        "启动并等待应成功；报告见 {}",
        report.display()
    );
    let text = std::fs::read_to_string(&report).expect("报告");
    assert!(text.contains("\"launched\": true"), "应真的启动过：{text}");
    assert!(text.contains("session:"), "应记录会话结果：{text}");
    assert!(text.contains("\"restored\": true"), "应自动恢复：{text}");
    assert_eq!(
        std::fs::read(root.join("game/Project/Saved/Config/Windows/Engine.ini")).expect("原件"),
        b"original=\r\n",
        "原件必须按字节恢复"
    );
    // 日志要有内容，而不只是目录
    let logs = root.join("logs");
    assert!(logs.is_dir(), "应写下日志目录：{}", logs.display());
    let mut log_text = String::new();
    for entry in std::fs::read_dir(&logs).expect("读日志目录") {
        let path = entry.expect("条目").path();
        if path.extension().and_then(|value| value.to_str()) == Some("log") {
            log_text.push_str(&std::fs::read_to_string(&path).expect("读日志"));
        }
    }
    assert!(log_text.contains("部署完成"), "日志应记录部署：{log_text}");
    assert!(log_text.contains("恢复完成"), "日志应记录恢复：{log_text}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn second_session_is_refused_while_the_first_holds_the_lock() {
    let _guard = SESSION_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    wait_for_free_session();
    let (root, preferences) = engine_fixture("session");
    let root_arg = root.to_string_lossy().into_owned();
    let preferences_arg = preferences.to_string_lossy().into_owned();
    let first_report = root.join("first.json");
    let second_report = root.join("second.json");

    let mut first = Command::new(binary())
        .args([
            "--engine",
            "run",
            "--game",
            "fixture",
            "--root",
            &root_arg,
            "--preferences",
            &preferences_arg,
            "--hold-ms",
            "1500",
            "--skip-launch",
            "--report",
            first_report.to_str().expect("路径"),
        ])
        .spawn()
        .expect("起第一个会话");
    std::thread::sleep(std::time::Duration::from_millis(500));

    let second = Command::new(binary())
        .args([
            "--engine",
            "run",
            "--game",
            "fixture",
            "--root",
            &root_arg,
            "--preferences",
            &preferences_arg,
            "--skip-launch",
            "--report",
            second_report.to_str().expect("路径"),
        ])
        .status()
        .expect("起第二个会话");
    assert_eq!(second.code(), Some(6), "第二个会话必须被拒（session-busy）");
    let text = std::fs::read_to_string(&second_report).expect("第二份报告");
    assert!(text.contains("session-busy"), "{text}");

    let status_text = std::fs::read_to_string(root.join("status.json")).expect("状态");
    assert!(
        status_text.contains("\"phase\":\"starting\""),
        "{status_text}"
    );

    let first_status = first.wait().expect("等第一个会话");
    assert_eq!(first_status.code(), Some(0), "第一个会话应正常结束");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn running_launcher_is_detected_and_reported() {
    let _guard = SESSION_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    wait_for_free_session();
    let (root, preferences) = engine_fixture("reuse");
    let root_arg = root.to_string_lossy().into_owned();
    let preferences_arg = preferences.to_string_lossy().into_owned();
    let mut decoy = Command::new("ping")
        .args(["-n", "4", "127.0.0.1"])
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("起 ping");
    std::thread::sleep(std::time::Duration::from_millis(300));

    let hit = root.join("hit.json");
    let status = run(&[
        "--engine",
        "run",
        "--game",
        "fixture",
        "--root",
        &root_arg,
        "--preferences",
        &preferences_arg,
        "--launcher-name",
        "PING",
        "--skip-launch",
        "--report",
        hit.to_str().expect("路径"),
    ]);
    assert_eq!(status.code(), Some(0));
    let status_text = std::fs::read_to_string(root.join("status.json")).expect("状态");
    assert!(status_text.contains("复用"), "应说明复用：{status_text}");

    let miss = root.join("miss.json");
    let status = run(&[
        "--engine",
        "run",
        "--game",
        "fixture",
        "--root",
        &root_arg,
        "--preferences",
        &preferences_arg,
        "--launcher-name",
        "NoSuchLauncherXyz",
        "--skip-launch",
        "--report",
        miss.to_str().expect("路径"),
    ]);
    assert_eq!(status.code(), Some(0));
    let status_text = std::fs::read_to_string(root.join("status.json")).expect("状态");
    assert!(!status_text.contains("复用"), "不该误报复用：{status_text}");

    let _ = decoy.kill();
    let _ = decoy.wait();
    let _ = std::fs::remove_dir_all(&root);
}
#[test]
fn nte_settings_match_old_script() {
    let text = r#"{"enabled":true,"engine":"Dlss","max_generated_frames":3,"preset":"C","optimized":2,"super_resolution":"LFp16","level":0,"directory":"dlssg_sm86\\logs","flow_resolution":"Middle","use_hudless":"Auto","runtime_mode":"Bundled","runtime_path":null,"custom_ini":null}"#;
    let settings: launcher_core::settings::Settings = serde_json::from_str(text).expect("解析设置");
    assert_eq!(settings.preset, launcher_core::settings::Preset::C);
    assert_eq!(settings.optimized, 2);
    assert_eq!(settings.level, 0);
    assert_eq!(
        settings.super_resolution,
        launcher_core::settings::SuperResolutionMode::LFp16
    );
    let ini = settings.render("test-identity").expect("生成诊断 INI");
    assert!(!ini.trim().is_empty(), "诊断 INI 不应为空");
    assert!(ini.contains("DlssgLauncher"), "{ini}");
}
