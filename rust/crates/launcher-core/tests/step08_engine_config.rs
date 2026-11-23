//! `Engine.ini` 准备与恢复的往返（PARITY C8 + C5）。
//!
//! 覆盖 `Prepare-EngineConfig` 的四个动作：只读副本、备份、链接、复用；
//! 再跑一遍 `restore`，确认原件按字节回来、只读副本能被删掉。

use std::path::{Path, PathBuf};

use launcher_core::SymlinkOps;
use launcher_core::engine_config::{
    EngineConfigSpec, RuntimeMarker, prepare_engine_config, read_source_text, runtime_copy_bytes,
};
use launcher_core::journal::{Journal, OperationKind, OperationState};
use launcher_core::journal_store::{DeploymentSession, load};
use launcher_core::ops::{LinkInfo, LinkKind};
use launcher_core::restore::{RestoreOptions, restore};
use launcher_core::testing::{FakeMutexOps, FakeSymlinkOps};

const SESSION: &str = "0123456789abcdef0123456789abcdef";
const SOURCE: &str = "[SystemSettings]\r\nr.Foo=1\r\n";
const ORIGINAL: &str = "original engine.ini\r\n";

struct Fixture {
    root: PathBuf,
    render: PathBuf,
    source: PathBuf,
    engine: PathBuf,
    runtime: PathBuf,
    backup: PathBuf,
    state: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn setup(name: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("dlssg-enginecfg-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let render = root.join("render");
    let state = root.join("state");
    std::fs::create_dir_all(&render).expect("渲染目录");
    std::fs::create_dir_all(&state).expect("状态目录");
    let source = root.join("Engine.source.ini");
    std::fs::write(&source, SOURCE).expect("配置源");
    Fixture {
        engine: render.join("Engine.ini"),
        runtime: root.join("dlssg_sm86-engine-1234.ini"),
        backup: root.join("Engine.backup.ini"),
        root,
        render,
        source,
        state,
    }
}

fn spec(fixture: &Fixture, marker: RuntimeMarker) -> EngineConfigSpec {
    EngineConfigSpec {
        engine_path: fixture.engine.clone(),
        runtime_path: fixture.runtime.clone(),
        backup_path: fixture.backup.clone(),
        source_path: fixture.source.clone(),
        marker,
    }
}

fn journal(fixture: &Fixture) -> Journal {
    let mut journal =
        Journal::from_json(&launcher_core::fixtures::read_text("journal/01-new.json"))
            .expect("样本");
    journal.game_id = "fixture".to_owned();
    journal.session = SESSION.to_owned();
    journal.render = fixture.render.to_string_lossy().into_owned();
    journal.engine = fixture.engine.to_string_lossy().into_owned();
    journal.runtime = fixture.runtime.to_string_lossy().into_owned();
    journal.backup = fixture.backup.to_string_lossy().into_owned();
    journal.original_existed = true;
    journal.original_hash = Some(launcher_core::path::text_hash_upper(ORIGINAL));
    journal.operations.clear();
    journal
}

fn journal_path(fixture: &Fixture) -> PathBuf {
    fixture
        .state
        .join(".state")
        .join("fixture")
        .join("active.json")
}

#[test]
fn runtime_copy_matches_the_marker_and_source_bytes() {
    let fixture = setup("copy");
    std::fs::write(&fixture.engine, ORIGINAL).expect("原件");
    let ops = FakeSymlinkOps::new();
    let mutex = FakeMutexOps::new();
    let mut session =
        DeploymentSession::open(&mutex, &fixture.state, &fixture.render, journal(&fixture))
            .expect("会话");

    let outcome = prepare_engine_config(
        &mut session,
        &ops,
        &spec(&fixture, RuntimeMarker::Wuthering),
        false,
    )
    .expect("准备");

    assert!(outcome.runtime_written && outcome.backup_moved && outcome.link_created);
    let bytes = std::fs::read(&fixture.runtime).expect("运行时副本");
    assert_eq!(
        String::from_utf8(bytes).expect("UTF-8"),
        "; WuWa-Configs launcher runtime copy v1\r\n[SystemSettings]\r\nr.Foo=1\r\n"
    );
    assert!(
        std::fs::metadata(&fixture.runtime)
            .expect("元数据")
            .permissions()
            .readonly(),
        "运行时副本必须是只读"
    );
    // 原件被移走、备份内容与原件一致
    assert!(!fixture.engine.exists() || ops.inspect(&fixture.engine).expect("查询").is_some());
    assert_eq!(
        std::fs::read_to_string(&fixture.backup).expect("备份"),
        ORIGINAL
    );

    let journal = session.journal();
    let kinds: Vec<OperationKind> = journal.operations.iter().map(|item| item.kind).collect();
    assert_eq!(
        kinds,
        vec![
            OperationKind::Runtime,
            OperationKind::Backup,
            OperationKind::EngineLink
        ]
    );
    assert!(
        journal
            .operations
            .iter()
            .all(|operation| operation.state == OperationState::Done)
    );
    let backup = &journal.operations[1];
    assert_eq!(backup.hash, launcher_core::path::text_hash_upper(ORIGINAL));
    let link = &journal.operations[2];
    assert_eq!(
        std::fs::canonicalize(&fixture.runtime).expect("规范化"),
        std::fs::canonicalize(&link.target).expect("规范化")
    );
    session.finish().expect("收尾");
}

#[test]
fn prepare_skips_backup_when_no_original_exists() {
    let fixture = setup("no-original");
    let ops = FakeSymlinkOps::new();
    let mutex = FakeMutexOps::new();
    let mut journal = journal(&fixture);
    journal.original_existed = false;
    journal.original_hash = None;
    let mut session =
        DeploymentSession::open(&mutex, &fixture.state, &fixture.render, journal).expect("会话");

    let outcome = prepare_engine_config(
        &mut session,
        &ops,
        &spec(&fixture, RuntimeMarker::Neverness),
        false,
    )
    .expect("准备");
    assert!(!outcome.backup_moved);
    assert!(outcome.runtime_written && outcome.link_created);
    assert!(!fixture.backup.exists());
    assert!(
        session
            .journal()
            .operations
            .iter()
            .all(|operation| operation.kind != OperationKind::Backup)
    );
    session.finish().expect("收尾");
}

#[test]
fn prepare_refuses_a_directory_at_the_engine_path_before_writing() {
    let fixture = setup("refuse-dir");
    std::fs::create_dir_all(&fixture.engine).expect("占位目录");
    let ops = FakeSymlinkOps::new();
    let mutex = FakeMutexOps::new();
    let mut session =
        DeploymentSession::open(&mutex, &fixture.state, &fixture.render, journal(&fixture))
            .expect("会话");

    let error = prepare_engine_config(
        &mut session,
        &ops,
        &spec(&fixture, RuntimeMarker::Neverness),
        false,
    )
    .expect_err("应拒绝");
    assert!(error.to_string().contains("链接或目录"), "实际：{error}");
    assert!(!fixture.runtime.exists(), "拒绝时不应留下运行时副本");
    assert!(session.journal().operations.is_empty(), "拒绝时不应写操作");
    session.finish().expect("收尾");
}

#[test]
fn second_prepare_reuses_the_engine_link() {
    let fixture = setup("reuse");
    std::fs::write(&fixture.engine, ORIGINAL).expect("原件");
    let ops = FakeSymlinkOps::new();
    let mutex = FakeMutexOps::new();
    let mut session =
        DeploymentSession::open(&mutex, &fixture.state, &fixture.render, journal(&fixture))
            .expect("会话");
    prepare_engine_config(
        &mut session,
        &ops,
        &spec(&fixture, RuntimeMarker::Neverness),
        false,
    )
    .expect("第一次准备");

    let outcome = prepare_engine_config(
        &mut session,
        &ops,
        &spec(&fixture, RuntimeMarker::Neverness),
        false,
    )
    .expect("第二次准备");
    assert!(outcome.link_reused);
    assert!(!outcome.link_created);
    assert_eq!(
        session
            .journal()
            .operations_of(OperationKind::EngineLink)
            .len(),
        1,
        "复用不应追加第二条链接记录"
    );
    session.finish().expect("收尾");
}

#[test]
fn dry_run_writes_nothing() {
    let fixture = setup("dry");
    std::fs::write(&fixture.engine, ORIGINAL).expect("原件");
    let ops = FakeSymlinkOps::new();
    let mutex = FakeMutexOps::new();
    let mut session =
        DeploymentSession::open(&mutex, &fixture.state, &fixture.render, journal(&fixture))
            .expect("会话");

    let outcome = prepare_engine_config(
        &mut session,
        &ops,
        &spec(&fixture, RuntimeMarker::Neverness),
        true,
    )
    .expect("演练");
    assert_eq!(outcome, Default::default());
    assert!(!fixture.runtime.exists());
    assert!(fixture.engine.is_file(), "演练不能移动原件");
    assert!(session.journal().operations.is_empty());
    session.finish().expect("收尾");
}

#[test]
fn prepare_then_restore_round_trips_and_removes_the_readonly_copy() {
    let fixture = setup("round-trip");
    std::fs::write(&fixture.engine, ORIGINAL).expect("原件");
    let ops = FakeSymlinkOps::new();
    let mutex = FakeMutexOps::new();
    let mut session =
        DeploymentSession::open(&mutex, &fixture.state, &fixture.render, journal(&fixture))
            .expect("会话");
    prepare_engine_config(
        &mut session,
        &ops,
        &spec(
            &fixture,
            RuntimeMarker::Steam {
                app_id: "2358720".to_owned(),
            },
        ),
        false,
    )
    .expect("准备");
    // 现场：链接存在（假链接层），原件在备份里
    assert!(
        std::fs::metadata(&fixture.runtime)
            .expect("副本")
            .permissions()
            .readonly()
    );
    assert!(!fixture.engine.exists());

    let outcome = restore(
        &mut session,
        &ops,
        &RestoreOptions::default(),
        &|_| Ok(()),
        &[],
    )
    .expect("恢复");
    assert!(outcome.is_complete(), "{:?}", outcome.conflicts);
    assert_eq!(
        std::fs::read_to_string(&fixture.engine).expect("原件"),
        ORIGINAL,
        "原件必须按字节恢复"
    );
    assert!(!fixture.runtime.exists(), "只读副本应被删除（先清属性）");
    assert!(!fixture.backup.exists(), "备份应被移回");
    session.finish().expect("收尾");
    assert!(load(&journal_path(&fixture)).expect("读记录").is_none());
}

#[test]
fn source_text_is_decoded_with_the_config_rules() {
    let fixture = setup("decode");
    // UTF-16 LE + BOM 的配置源
    let mut bytes = vec![0xFF, 0xFE];
    for unit in SOURCE.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    std::fs::write(&fixture.source, &bytes).expect("UTF-16 源");
    assert_eq!(read_source_text(&fixture.source).expect("解码"), SOURCE);
    assert_eq!(
        String::from_utf8(runtime_copy_bytes(
            &RuntimeMarker::Wuthering,
            &read_source_text(&fixture.source).expect("解码")
        ))
        .expect("UTF-8"),
        format!("; WuWa-Configs launcher runtime copy v1\r\n{SOURCE}")
    );
}

#[test]
fn engine_link_must_target_the_runtime_copy() {
    let fixture = setup("wrong-link");
    std::fs::write(&fixture.engine, ORIGINAL).expect("原件");
    let ops = FakeSymlinkOps::new().with_link(
        &fixture.engine,
        LinkInfo {
            kind: LinkKind::SymbolicLink,
            target: Path::new(r"D:\elsewhere\other.ini").to_path_buf(),
            target_is_dir: false,
        },
    );
    let mutex = FakeMutexOps::new();
    let mut session =
        DeploymentSession::open(&mutex, &fixture.state, &fixture.render, journal(&fixture))
            .expect("会话");

    let error = prepare_engine_config(
        &mut session,
        &ops,
        &spec(&fixture, RuntimeMarker::Neverness),
        false,
    )
    .expect_err("应拒绝");
    assert!(error.to_string().contains("已被占用"), "实际：{error}");
    session.finish().expect("收尾");
}
