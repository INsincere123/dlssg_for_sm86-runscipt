//! Phase 1 步 7：部署记录落盘与会话。
//!
//! 对照 `DeploymentJournal.ps1:28-49`（原子落盘）、`:14-26`（父目录检查）、
//! `:72-102`（互斥）与 `:207-259`（阶段与操作状态）。

use std::path::{Path, PathBuf};
use std::time::Duration;

use launcher_core::Error;
use launcher_core::journal::{Journal, OperationKind, OperationState, Phase};
use launcher_core::journal_store::{
    self, configuration_mutex_name, deployment_mutex_name, identity,
};
use launcher_core::testing::FakeMutexOps;

fn scratch(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("dlssg-step07-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("创建临时目录");
    directory
}

fn fixture_journal(relative: &str) -> Journal {
    Journal::from_json(&launcher_core::fixtures::read_text(relative)).expect("解析样本")
}

#[test]
fn save_and_load_round_trips_every_field() {
    let directory = scratch("roundtrip");
    let path = directory.join("active.json");
    let journal = fixture_journal("journal/04-partial-failure.json");

    journal_store::save_atomic(&path, &journal).expect("落盘");
    let loaded = journal_store::load(&path).expect("读回").expect("应存在");
    assert_eq!(loaded, journal);
    assert_eq!(
        loaded.operations_of(OperationKind::Storage)[0].state,
        OperationState::Intent
    );

    // 落盘后不应留下临时文件
    let leftovers: Vec<String> = std::fs::read_dir(&directory)
        .expect("列目录")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "残留临时文件：{leftovers:?}");
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn load_returns_none_for_missing_journal() {
    let directory = scratch("missing");
    assert!(
        journal_store::load(&directory.join("nope.json"))
            .expect("读")
            .is_none()
    );
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn journal_path_must_be_an_ordinary_file() {
    let directory = scratch("dir");
    let path = directory.join("active.json");
    std::fs::create_dir_all(&path).expect("建目录占位");
    let journal = fixture_journal("journal/01-new.json");
    let error = journal_store::save_atomic(&path, &journal).expect_err("应拒绝");
    assert!(
        error.to_string().contains("not an ordinary file"),
        "实际：{error}"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn identity_matches_the_powershell_formula() {
    // 与 PS 相同的输入必须得到相同的 identity：归一化后再 SHA256（大写十六进制）
    let render = Path::new(r"D:\SteamLibrary\steamapps\common\NBA 2K27");
    let normalized = launcher_core::path::PathContext::current()
        .expect("路径上下文")
        .normalize(render.to_str().expect("utf8"))
        .expect("归一化");
    let expected = launcher_core::path::sha256_upper(normalized.as_bytes());
    assert_eq!(identity(render).expect("identity"), expected);
    assert_eq!(
        deployment_mutex_name(render).expect("互斥名"),
        format!("Global\\DLSSG-Deployment-{expected}")
    );
    assert_eq!(
        configuration_mutex_name(render).expect("互斥名"),
        format!("Global\\DLSSG-Config-{expected}")
    );
}

#[test]
fn session_persists_every_transition() {
    let directory = scratch("session");
    let ops = FakeMutexOps::new();
    let mut journal = fixture_journal("journal/01-new.json");
    journal.session = "0123456789abcdef0123456789abcdef".to_owned();

    let render = Path::new(r"D:\Games\Fixture");
    let mut session = journal_store::DeploymentSession::open(&ops, &directory, render, journal)
        .expect("打开会话");
    let path = session.path().to_path_buf();

    // 打开即落盘
    assert!(path.is_file(), "打开会话应立即写入记录");
    assert_eq!(ops.acquisitions.borrow().len(), 2, "应抢到两个互斥");
    assert!(
        session
            .mutex_names()
            .0
            .starts_with("Global\\DLSSG-Deployment-")
    );
    let config_name = configuration_mutex_name(Path::new(&session.journal().engine)).unwrap();
    assert_eq!(session.mutex_names().1, Some(config_name.as_str()));

    session.set_phase(Phase::Recovering).expect("改阶段");
    let reloaded = journal_store::load(&path).expect("读").expect("存在");
    assert_eq!(reloaded.phase, Phase::Recovering);

    let index = session
        .start_operation(OperationKind::EngineLink, r"D:\Games\Fixture\Engine.ini")
        .expect("记录 intent");
    session
        .set_operation_target(index, r"D:\Temp\runtime.ini")
        .expect("目标");
    let reloaded = journal_store::load(&path).expect("读").expect("存在");
    assert_eq!(reloaded.operations.len(), 1);
    assert_eq!(reloaded.operations[0].state, OperationState::Intent);
    assert_eq!(reloaded.operations[0].target, r"D:\Temp\runtime.ini");

    session.complete_operation(index).expect("完成");
    session.mark_restored(index).expect("已恢复");
    let reloaded = journal_store::load(&path).expect("读").expect("存在");
    assert_eq!(reloaded.operations[0].state, OperationState::Restored);
    assert!(!reloaded.has_pending_operations());

    session.finish().expect("收尾");
    assert!(!path.exists(), "收尾后记录应删除");
    assert_eq!(ops.releases.borrow().len(), 2, "两个互斥都应释放");
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn second_session_for_the_same_game_is_refused() {
    let directory = scratch("locked");
    let ops = FakeMutexOps::new();
    let mut journal = fixture_journal("journal/01-new.json");
    journal.session = "0123456789abcdef0123456789abcdef".to_owned();
    let render = Path::new(r"D:\Games\Fixture");
    let first = journal_store::DeploymentSession::open(&ops, &directory, render, journal.clone())
        .expect("第一个会话");

    // FakeMutexOps 记录同名互斥已被占用 → 第二个会话必须失败
    let error = journal_store::DeploymentSession::open(&ops, &directory, render, journal)
        .expect_err("第二个会话应被拒绝");
    assert!(
        error.to_string().contains("部署/恢复正在进行"),
        "实际：{error}"
    );
    first.finish().expect("收尾");
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn shared_engine_ini_blocks_a_second_render_directory() {
    let directory = scratch("shared-engine");
    let ops = FakeMutexOps::new();
    let mut first = fixture_journal("journal/01-new.json");
    first.session = "0123456789abcdef0123456789abcdef".to_owned();
    first.engine = r"C:\Saves\Game\Engine.ini".to_owned();
    let mut second = first.clone();
    second.game_id = "other".to_owned();
    second.engine = r"c:\saves\game\Saved\..\ENGINE.ini".to_owned();
    let first = journal_store::DeploymentSession::open(
        &ops,
        &directory,
        Path::new(r"D:\Games\First"),
        first,
    )
    .expect("第一个会话");
    let error = journal_store::DeploymentSession::open(
        &ops,
        &directory,
        Path::new(r"D:\Games\Second"),
        second,
    )
    .expect_err("同一 Engine.ini 已被占用");
    let name = configuration_mutex_name(Path::new(r"c:\saves\game\engine.ini")).unwrap();
    assert!(matches!(error, Error::MutexTimeout { name: actual, .. } if actual == name));
    assert!(
        !directory
            .join(journal_store::state_relative_path("other"))
            .exists()
    );
    first.finish().expect("收尾");
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn resume_refuses_to_replace_a_changed_record() {
    let directory = scratch("changed-record");
    let ops = FakeMutexOps::new();
    let mut before = fixture_journal("journal/01-new.json");
    before.session = "0123456789abcdef0123456789abcdef".to_owned();
    let path = directory.join(journal_store::state_relative_path(&before.game_id));
    let mut after = before.clone();
    after.phase = Phase::Recovering;
    journal_store::save_atomic(&path, &after).unwrap();

    let error = journal_store::DeploymentSession::resume(&ops, path.clone(), before)
        .expect_err("记录快照已过期");
    assert!(error.to_string().contains("记录已变化"), "{error}");
    assert_eq!(journal_store::load(&path).unwrap(), Some(after));
    let _ = std::fs::remove_dir_all(&directory);
}

#[cfg(windows)]
#[test]
fn delete_failure_keeps_the_record_available_for_retry() {
    use std::os::windows::fs::OpenOptionsExt;

    let directory = scratch("delete-failure");
    let mut journal = fixture_journal("journal/01-new.json");
    journal.session = "0123456789abcdef0123456789abcdef".to_owned();
    let ops = FakeMutexOps::new();
    let session = journal_store::DeploymentSession::open(
        &ops,
        &directory,
        Path::new(&journal.render),
        journal.clone(),
    )
    .unwrap();
    let path = session.path().to_path_buf();
    let handle = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0x3)
        .open(&path)
        .unwrap();
    session.finish().expect_err("占用句柄未允许删除");
    assert_eq!(journal_store::load(&path).unwrap(), Some(journal.clone()));

    drop(handle);
    journal_store::DeploymentSession::resume(&ops, path.clone(), journal)
        .unwrap()
        .finish()
        .unwrap();
    assert!(!path.exists());
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn dlssg_only_session_refuses_engine_operations() {
    let directory = scratch("dlssgonly");
    let ops = FakeMutexOps::new();
    let mut journal = fixture_journal("journal/07-no-original.json");
    journal.session = "0123456789abcdef0123456789abcdef".to_owned();
    journal.deployment_mode = Some(launcher_core::journal::DeploymentMode::DlssgOnly);
    let mut session = journal_store::DeploymentSession::open(
        &ops,
        &directory,
        Path::new(r"D:\Games\Fixture"),
        journal,
    )
    .expect("打开会话");
    assert_eq!(session.mutex_names().1, None);

    for kind in [
        OperationKind::EngineLink,
        OperationKind::Backup,
        OperationKind::Runtime,
    ] {
        let error = session
            .start_operation(kind, r"D:\Games\Fixture\Engine.ini")
            .expect_err("DLSSG 模式应拒绝");
        assert!(error.to_string().contains("仅 DLSSG 模式"), "实际：{error}");
    }
    // 资源类操作仍然允许
    session
        .start_operation(OperationKind::Link, r"D:\Games\Fixture\version.dll")
        .expect("允许");
    session.finish().expect("收尾");
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn invalid_session_identifier_is_refused_before_writing() {
    let directory = scratch("bad-session");
    let ops = FakeMutexOps::new();
    let mut journal = fixture_journal("journal/01-new.json");
    journal.session = "not-a-guid".to_owned();
    let error = journal_store::DeploymentSession::open(
        &ops,
        &directory,
        Path::new(r"D:\Games\Fixture"),
        journal,
    )
    .expect_err("应拒绝非法会话");
    assert!(matches!(error, Error::Invalid { .. }));
    assert!(ops.acquisitions.borrow().is_empty(), "校验失败不应抢互斥");
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn operation_index_is_checked() {
    let directory = scratch("index");
    let ops = FakeMutexOps::new();
    let mut journal = fixture_journal("journal/01-new.json");
    journal.session = "0123456789abcdef0123456789abcdef".to_owned();
    let mut session = journal_store::DeploymentSession::open(
        &ops,
        &directory,
        Path::new(r"D:\Games\Fixture"),
        journal,
    )
    .expect("打开会话");
    let error = session.complete_operation(7).expect_err("越界应报错");
    assert!(
        error.to_string().contains("没有第 7 条操作"),
        "实际：{error}"
    );
    session.finish().expect("收尾");
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn mutex_wait_is_zero_for_the_session() {
    let directory = scratch("wait");
    let ops = FakeMutexOps::new();
    let mut journal = fixture_journal("journal/01-new.json");
    journal.session = "0123456789abcdef0123456789abcdef".to_owned();
    let session = journal_store::DeploymentSession::open(
        &ops,
        &directory,
        Path::new(r"D:\Games\Fixture"),
        journal,
    )
    .expect("打开会话");
    for (_, wait) in ops.acquisitions.borrow().iter() {
        assert_eq!(*wait, Duration::from_millis(0), "会话互斥应 WaitOne(0)");
    }
    session.finish().expect("收尾");
    let _ = std::fs::remove_dir_all(&directory);
}
