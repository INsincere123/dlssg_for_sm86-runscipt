//! 诊断 INI 部署与拆除的往返（PARITY C7 部署侧 + restore 的 diagnostic 分支）。

use std::path::PathBuf;

use launcher_core::SymlinkOps;
use launcher_core::diagnostic::{DiagnosticSpec, deploy_diagnostic};
use launcher_core::journal::{Journal, OperationKind, OperationState};
use launcher_core::journal_store::{DeploymentSession, load};
use launcher_core::ops::{LinkInfo, LinkKind};
use launcher_core::restore::{RestoreOptions, restore};
use launcher_core::settings::{Preset, SuperResolutionMode, render_diagnostic};
use launcher_core::testing::{FakeMutexOps, FakeSymlinkOps};

const SESSION: &str = "0123456789abcdef0123456789abcdef";

struct Fixture {
    root: PathBuf,
    render: PathBuf,
    state: PathBuf,
    temp: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn setup(name: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("dlssg-diag-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let render = root.join("render");
    let state = root.join("state");
    std::fs::create_dir_all(&render).expect("渲染目录");
    std::fs::create_dir_all(&state).expect("状态目录");
    Fixture {
        temp: root.join("dlssg_sm86-nte-4242.ini"),
        root,
        render,
        state,
    }
}

fn journal(fixture: &Fixture) -> Journal {
    let mut journal =
        Journal::from_json(&launcher_core::fixtures::read_text("journal/01-new.json"))
            .expect("样本");
    journal.game_id = "fixture".to_owned();
    journal.session = SESSION.to_owned();
    journal.render = fixture.render.to_string_lossy().into_owned();
    journal.engine = String::new();
    journal.runtime = String::new();
    journal.backup = String::new();
    journal.original_existed = false;
    journal.original_hash = None;
    journal.operations.clear();
    journal
}

fn spec(fixture: &Fixture) -> DiagnosticSpec {
    let template = launcher_core::fixtures::read_text("templates/neverness/0.4.2.ini");
    DiagnosticSpec {
        render_dir: fixture.render.clone(),
        temp_path: fixture.temp.clone(),
        text: render_diagnostic(&template, SuperResolutionMode::MInt8, Preset::C, 3, 3),
    }
}

#[test]
fn deploy_writes_temp_file_and_link_with_text_hash() {
    let fixture = setup("deploy");
    let ops = FakeSymlinkOps::new();
    let mutex = FakeMutexOps::new();
    let mut session =
        DeploymentSession::open(&mutex, &fixture.state, &fixture.render, journal(&fixture))
            .expect("会话");
    let spec = spec(&fixture);

    let outcome = deploy_diagnostic(&mut session, &ops, &spec, false).expect("部署");
    assert!(outcome.temp_written && outcome.link_created && !outcome.link_reused);
    assert_eq!(
        std::fs::read_to_string(&fixture.temp).expect("临时文件"),
        spec.text
    );
    // 记录里 diagnostic 操作用的是文本哈希（不是文件哈希）
    let diagnostic = session.journal().operations_of(OperationKind::Diagnostic);
    assert_eq!(diagnostic.len(), 1);
    assert_eq!(
        diagnostic[0].hash,
        launcher_core::path::text_hash_upper(&spec.text)
    );
    assert_eq!(diagnostic[0].state, OperationState::Done);
    // 链接指向临时文件
    let link = spec.link_path();
    let info = ops.inspect(&link).expect("查询").expect("应有链接");
    assert_eq!(info.target, fixture.temp);
    assert!(!info.target_is_dir);
    session.finish().expect("收尾");
}

#[test]
fn existing_temp_file_is_refused_before_any_operation() {
    let fixture = setup("temp-exists");
    std::fs::write(&fixture.temp, "leftover").expect("残留临时文件");
    let ops = FakeSymlinkOps::new();
    let mutex = FakeMutexOps::new();
    let mut session =
        DeploymentSession::open(&mutex, &fixture.state, &fixture.render, journal(&fixture))
            .expect("会话");

    let error = deploy_diagnostic(&mut session, &ops, &spec(&fixture), false).expect_err("应拒绝");
    assert!(error.to_string().contains("拒绝覆盖"), "实际：{error}");
    assert!(session.journal().operations.is_empty(), "不应写记录");
    assert_eq!(
        std::fs::read_to_string(&fixture.temp).expect("原文件"),
        "leftover",
        "残留文件不能被改写"
    );
    session.finish().expect("收尾");
}

#[test]
fn existing_matching_link_is_reused() {
    let fixture = setup("reuse");
    let mutex = FakeMutexOps::new();
    let mut session =
        DeploymentSession::open(&mutex, &fixture.state, &fixture.render, journal(&fixture))
            .expect("会话");
    let spec = spec(&fixture);
    deploy_diagnostic(&mut session, &FakeSymlinkOps::new(), &spec, false).expect("首次部署");

    // 第二次：临时文件已存在会被拒绝，所以先删掉临时文件、只留链接
    let ops = FakeSymlinkOps::new().with_link(
        spec.link_path(),
        LinkInfo {
            kind: LinkKind::SymbolicLink,
            target: spec.temp_path.clone(),
            target_is_dir: false,
        },
    );
    std::fs::remove_file(&fixture.temp).expect("删临时文件");
    let outcome = deploy_diagnostic(&mut session, &ops, &spec, false).expect("复用");
    assert!(outcome.link_reused && !outcome.link_created, "{outcome:?}");
    assert_eq!(
        session.journal().operations_of(OperationKind::Link).len(),
        1,
        "复用不应追加第二条链接记录"
    );
    session.finish().expect("收尾");
}

#[test]
fn ordinary_file_at_link_position_is_refused() {
    let fixture = setup("link-occupied");
    std::fs::write(fixture.render.join("dlssg_sm86.ini"), "user ini").expect("占位文件");
    let ops = FakeSymlinkOps::new();
    let mutex = FakeMutexOps::new();
    let mut session =
        DeploymentSession::open(&mutex, &fixture.state, &fixture.render, journal(&fixture))
            .expect("会话");

    let error = deploy_diagnostic(&mut session, &ops, &spec(&fixture), false).expect_err("应拒绝");
    assert!(error.to_string().contains("拒绝覆盖"), "实际：{error}");
    assert_eq!(
        std::fs::read_to_string(fixture.render.join("dlssg_sm86.ini")).expect("占位文件"),
        "user ini"
    );
    session.finish().expect("收尾");
}

#[test]
fn dry_run_touches_nothing() {
    let fixture = setup("dry");
    let ops = FakeSymlinkOps::new();
    let mutex = FakeMutexOps::new();
    let mut session =
        DeploymentSession::open(&mutex, &fixture.state, &fixture.render, journal(&fixture))
            .expect("会话");

    let outcome = deploy_diagnostic(&mut session, &ops, &spec(&fixture), true).expect("演练");
    assert_eq!(outcome, Default::default());
    assert!(!fixture.temp.exists());
    assert!(session.journal().operations.is_empty());
    session.finish().expect("收尾");
}

#[test]
fn restore_removes_temp_file_and_link() {
    let fixture = setup("round-trip");
    let ops = FakeSymlinkOps::new();
    let mutex = FakeMutexOps::new();
    let mut session =
        DeploymentSession::open(&mutex, &fixture.state, &fixture.render, journal(&fixture))
            .expect("会话");
    let spec = spec(&fixture);
    deploy_diagnostic(&mut session, &ops, &spec, false).expect("部署");

    let outcome = restore(
        &mut session,
        &ops,
        &RestoreOptions::default(),
        &|_| Ok(()),
        &[],
    )
    .expect("恢复");
    assert!(outcome.is_complete(), "{:?}", outcome.conflicts);
    assert!(!fixture.temp.exists(), "临时文件应删除");
    assert!(
        ops.inspect(&spec.link_path()).expect("查询").is_none(),
        "链接应拆除"
    );
    assert!(
        load(
            &fixture
                .state
                .join(".state")
                .join("fixture")
                .join("active.json")
        )
        .expect("读记录")
        .is_none()
    );
    session.finish().expect("收尾");
}

#[test]
fn tampered_temp_file_is_preserved_on_restore() {
    let fixture = setup("tampered");
    let ops = FakeSymlinkOps::new();
    let mutex = FakeMutexOps::new();
    let mut session =
        DeploymentSession::open(&mutex, &fixture.state, &fixture.render, journal(&fixture))
            .expect("会话");
    let spec = spec(&fixture);
    deploy_diagnostic(&mut session, &ops, &spec, false).expect("部署");
    std::fs::write(&fixture.temp, "someone edited this").expect("篡改");

    let error = restore(
        &mut session,
        &ops,
        &RestoreOptions::default(),
        &|_| Ok(()),
        &[],
    )
    .expect_err("应报冲突");
    assert!(
        error.to_string().contains("File hash changed"),
        "实际：{error}"
    );
    assert!(fixture.temp.exists(), "被改过的临时文件必须保留");
    session.finish().expect("收尾");
}
