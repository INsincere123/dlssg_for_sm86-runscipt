//! `DlssgLauncher --engine check` 的验收：无控制台窗口、报告可读、参数校验（M2-02）。

use std::path::PathBuf;
use std::sync::Mutex;

// 单会话是机器级互斥：这两个用例必须串行，否则会互相抢锁。
static SESSION_LOCK: Mutex<()> = Mutex::new(());
use std::process::Command;

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
    assert!(
        text.contains(&format!("\"pid\": {}", std::process::id())) || text.contains("\"pid\": ")
    );
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn unknown_arguments_exit_with_usage_code() {
    assert_eq!(run(&["--nope"]).code(), Some(2));
    assert_eq!(run(&["--engine"]).code(), Some(2));
    assert_eq!(run(&["--engine", "bogus"]).code(), Some(2));
}

#[test]
fn run_and_restore_report_not_implemented_for_now() {
    // M2-03/M2-04/M2-05 接上之前，这两个子命令必须明确失败，而不是假装成功。
    assert_eq!(
        run(&["--engine", "run"]).code(),
        Some(2),
        "缺 --game 应是参数错误"
    );
    assert_eq!(run(&["--engine", "restore"]).code(), Some(3));
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
    // 本会话已是高完整性：runas 不会弹 UAC，子进程同样高完整性。
    // 若在普通权限环境跑这个用例，会弹 UAC 或返回 5（取消），因此只断言"有结果"。
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
        eprintln!("跳过：UAC 被取消（{text}）");
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
fn second_session_is_refused_while_the_first_holds_the_lock() {
    let _guard = SESSION_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let directory = scratch("session");
    let first_report = directory.join("first.json");
    let second_report = directory.join("second.json");

    let mut first = Command::new(binary())
        .args([
            "--engine",
            "run",
            "--game",
            "fixture",
            "--hold-ms",
            "1500",
            "--report",
            first_report.to_str().expect("路径"),
        ])
        .spawn()
        .expect("起第一个会话");
    // 等第一个会话拿到锁并写好状态
    std::thread::sleep(std::time::Duration::from_millis(400));

    let second = Command::new(binary())
        .args([
            "--engine",
            "run",
            "--game",
            "fixture",
            "--report",
            second_report.to_str().expect("路径"),
        ])
        .status()
        .expect("起第二个会话");
    assert_eq!(second.code(), Some(6), "第二个会话必须被拒（session-busy）");
    let text = std::fs::read_to_string(&second_report).expect("第二份报告");
    assert!(text.contains("session-busy"), "{text}");

    // 第一个会话仍在跑：它的 status.json 必须能读到阶段
    let status = directory.join("status.json");
    let status_text = std::fs::read_to_string(&status).expect("状态文件");
    assert!(
        status_text.contains("\"phase\":\"starting\""),
        "{status_text}"
    );

    let first_status = first.wait().expect("等第一个会话");
    assert_eq!(first_status.code(), Some(0));
    let finished = std::fs::read_to_string(&status).expect("状态文件");
    assert!(finished.contains("\"phase\":\"finished\""), "{finished}");
    let _ = std::fs::remove_dir_all(&directory);
}
#[test]
fn running_launcher_is_detected_and_reported() {
    let _guard = SESSION_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let directory = scratch("reuse");
    // 真实存在的外部进程：ping 会活 ~3 秒，用它的名字验证检测。
    let mut decoy = Command::new("ping")
        .args(["-n", "4", "127.0.0.1"])
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("起 ping");
    std::thread::sleep(std::time::Duration::from_millis(300));

    let hit = directory.join("hit.json");
    let status = run(&[
        "--engine",
        "run",
        "--game",
        "fixture",
        "--launcher-name",
        "PING",
        "--report",
        hit.to_str().expect("路径"),
    ]);
    assert_eq!(status.code(), Some(0));
    let text = std::fs::read_to_string(&hit).expect("报告");
    assert!(
        text.contains("\"reuse_launcher\": true"),
        "应检测到启动器：{text}"
    );
    // 最终状态是"结束"；复用的结论在报告里（状态文件只保留最后阶段）。
    let status_text = std::fs::read_to_string(directory.join("status.json")).expect("状态");
    assert!(
        status_text.contains("\"phase\":\"finished\""),
        "状态文件应记录最终阶段：{status_text}"
    );

    let miss = directory.join("miss.json");
    let status = run(&[
        "--engine",
        "run",
        "--game",
        "fixture",
        "--launcher-name",
        "NoSuchLauncherXyz",
        "--report",
        miss.to_str().expect("路径"),
    ]);
    assert_eq!(status.code(), Some(0));
    let text = std::fs::read_to_string(&miss).expect("报告");
    assert!(
        text.contains("\"reuse_launcher\": false"),
        "不应误报复用：{text}"
    );

    let _ = decoy.kill();
    let _ = decoy.wait();
    let _ = std::fs::remove_dir_all(&directory);
}
