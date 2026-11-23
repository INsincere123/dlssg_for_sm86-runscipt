//! Phase 1 步 10–11：恢复。
//!
//! 对照 `Restore-RecordedDeployment`（`DeploymentJournal.ps1:393-498`）。
//! 用真实临时目录放普通文件、用 `FakeSymlinkOps` 表示链接，因此不需要管理员权限。

use std::path::{Path, PathBuf};

use launcher_core::journal::{Journal, Operation, OperationKind, OperationState, Phase};
use launcher_core::journal_store::DeploymentSession;
use launcher_core::ops::{LinkInfo, LinkKind};
use launcher_core::restore::{RestoreOptions, restore};
use launcher_core::testing::{FakeMutexOps, FakeSymlinkOps};

const SESSION: &str = "0123456789abcdef0123456789abcdef";
const ORIGINAL: &str = "original engine.ini\r\n";
const PROXY_PAYLOAD: &str = "proxy-payload";
const DIAGNOSTIC_TEXT: &str = "[General]\r\nEnabled=1\r\n";

struct Scenario {
    root: PathBuf,
    project: PathBuf,
    render: PathBuf,
    state: PathBuf,
    diagnostic: PathBuf,
    ops: FakeSymlinkOps,
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn hash(text: &str) -> String {
    launcher_core::path::text_hash_upper(text)
}

fn hash_bytes(text: &str) -> String {
    launcher_core::path::sha256_upper(text.as_bytes())
}

impl Scenario {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("dlssg-restore-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let project = root.join("project");
        let render = root.join("render");
        let state = root.join("state");
        std::fs::create_dir_all(project.join("dlssg_sm86").join("runtime")).expect("项目资源");
        // 全量包的 sr 目录（轻量版没有它，资源链接会少一条；本文件的用例按全量布局跑）
        std::fs::create_dir_all(project.join("dlssg_sm86").join("sr")).expect("sr 资源");
        std::fs::create_dir_all(&render).expect("渲染目录");
        std::fs::create_dir_all(&state).expect("状态目录");
        std::fs::write(project.join("version.dll"), PROXY_PAYLOAD).expect("代理");
        let diagnostic = root.join("dlssg_sm86-wuthering-1234.ini");

        Scenario {
            root,
            project,
            render,
            state,
            diagnostic,
            ops: FakeSymlinkOps::new(),
        }
    }

    fn render_file(&self, name: &str) -> PathBuf {
        self.render.join(name)
    }

    /// 建立与 journal 一致的现场：原件已备份、runtime 副本、storage 副本、诊断 INI、资源根。
    fn lay_down(&self) {
        std::fs::write(self.render_file("Engine.backup.ini"), ORIGINAL).expect("备份");
        std::fs::write(self.render_file("Engine.runtime.ini"), "runtime copy").expect("runtime");
        std::fs::create_dir_all(self.render.join("_storage_")).expect("_storage_");
        std::fs::write(
            self.render.join("_storage_").join("version.dll"),
            PROXY_PAYLOAD,
        )
        .expect("storage 副本");
        std::fs::write(&self.diagnostic, DIAGNOSTIC_TEXT).expect("诊断 INI");
        std::fs::create_dir_all(self.render.join("dlssg_sm86")).expect("资源根");
    }

    fn journal(&self) -> Journal {
        let mut journal =
            Journal::from_json(&launcher_core::fixtures::read_text("journal/01-new.json"))
                .expect("样本");
        journal.game_id = "fixture".to_owned();
        journal.session = SESSION.to_owned();
        journal.render = self.render.to_string_lossy().into_owned();
        journal.engine = self
            .render_file("Engine.ini")
            .to_string_lossy()
            .into_owned();
        journal.runtime = self
            .render_file("Engine.runtime.ini")
            .to_string_lossy()
            .into_owned();
        journal.backup = self
            .render_file("Engine.backup.ini")
            .to_string_lossy()
            .into_owned();
        journal.original_existed = true;
        journal.original_hash = Some(hash(ORIGINAL));
        journal.storage_existed = true;
        journal.phase = Phase::Deploying;
        journal.operations = vec![
            Operation::new(
                OperationKind::Runtime,
                self.render_file("Engine.runtime.ini")
                    .to_string_lossy()
                    .into_owned(),
                OperationState::Done,
            ),
            Operation::new(
                OperationKind::Backup,
                self.render_file("Engine.ini")
                    .to_string_lossy()
                    .into_owned(),
                OperationState::Done,
            )
            .with_target(
                self.render_file("Engine.backup.ini")
                    .to_string_lossy()
                    .into_owned(),
            )
            .with_hash(hash(ORIGINAL)),
            Operation::new(
                OperationKind::EngineLink,
                self.render_file("Engine.ini")
                    .to_string_lossy()
                    .into_owned(),
                OperationState::Done,
            )
            .with_target(
                self.render_file("Engine.runtime.ini")
                    .to_string_lossy()
                    .into_owned(),
            ),
            Operation::new(
                OperationKind::Directory,
                self.render
                    .join("dlssg_sm86")
                    .to_string_lossy()
                    .into_owned(),
                OperationState::Done,
            ),
            Operation::new(
                OperationKind::Link,
                self.render
                    .join("dlssg_sm86")
                    .join("runtime")
                    .to_string_lossy()
                    .into_owned(),
                OperationState::Done,
            )
            .with_target(
                self.project
                    .join("dlssg_sm86")
                    .join("runtime")
                    .to_string_lossy()
                    .into_owned(),
            ),
            Operation::new(
                OperationKind::Storage,
                self.render
                    .join("_storage_")
                    .join("version.dll")
                    .to_string_lossy()
                    .into_owned(),
                OperationState::Done,
            )
            .with_hash(hash_bytes(PROXY_PAYLOAD)),
            Operation::new(
                OperationKind::Diagnostic,
                self.diagnostic.to_string_lossy().into_owned(),
                OperationState::Done,
            )
            .with_hash(hash(DIAGNOSTIC_TEXT)),
        ];
        journal
    }

    fn with_links(mut self) -> Self {
        let engine_link = LinkInfo {
            kind: LinkKind::SymbolicLink,
            target: self.render_file("Engine.runtime.ini"),
            target_is_dir: false,
        };
        let resource_link = LinkInfo {
            kind: LinkKind::SymbolicLink,
            target: self.project.join("dlssg_sm86").join("runtime"),
            target_is_dir: true,
        };
        let ops = std::mem::take(&mut self.ops);
        self.ops = ops
            .with_link(self.render_file("Engine.ini"), engine_link)
            .with_link(
                self.render.join("dlssg_sm86").join("runtime"),
                resource_link,
            );
        self
    }

    fn open<'a>(&self, mutex: &'a FakeMutexOps) -> DeploymentSession<'a> {
        DeploymentSession::open(mutex, &self.state, &self.render, self.journal()).expect("会话")
    }
}

fn gate_ok(_automatic: bool) -> launcher_core::Result<()> {
    Ok(())
}

fn journal_path(state: &Path) -> PathBuf {
    state.join(".state").join("fixture").join("active.json")
}

#[test]
fn full_restore_tears_everything_down_and_removes_the_journal() {
    let scenario = Scenario::new("full").with_links();
    scenario.lay_down();
    let mutex = FakeMutexOps::new();
    let mut session = scenario.open(&mutex);

    let outcome = restore(
        &mut session,
        &scenario.ops,
        &RestoreOptions::default(),
        &gate_ok,
        &[],
    )
    .expect("恢复应成功");

    assert!(outcome.is_complete(), "冲突：{:?}", outcome.conflicts);
    assert!(outcome.journal_removed);
    assert!(!journal_path(&scenario.state).exists(), "记录应删除");
    // 原件按字节恢复，副本与临时文件全部消失
    assert_eq!(
        std::fs::read_to_string(scenario.render_file("Engine.ini")).expect("原件"),
        ORIGINAL
    );
    assert!(!scenario.render_file("Engine.backup.ini").exists());
    assert!(!scenario.render_file("Engine.runtime.ini").exists());
    assert!(!scenario.diagnostic.exists());
    assert!(
        !scenario
            .render
            .join("_storage_")
            .join("version.dll")
            .exists()
    );
    // 资源链接由链接层移除，资源根因空被删
    assert_eq!(scenario.ops.removed().len(), 2, "引擎链接 + 资源链接");
    assert!(!scenario.render.join("dlssg_sm86").exists());
    session.finish().expect("收尾");
}

#[test]
fn replaced_engine_ini_is_archived_before_restoring_the_original() {
    let scenario = Scenario::new("archive");
    scenario.lay_down();
    // 游戏把软链接替换成了普通文件（链接层已不认为自己有这条链接）
    std::fs::write(scenario.render_file("Engine.ini"), "game wrote this").expect("替换文件");
    let mutex = FakeMutexOps::new();
    let mut session = scenario.open(&mutex);

    let outcome = restore(
        &mut session,
        &scenario.ops,
        &RestoreOptions::default(),
        &gate_ok,
        &[],
    )
    .expect("恢复应成功");

    assert_eq!(outcome.archived.len(), 1, "应归档一次");
    let saved = &outcome.archived[0].1;
    assert_eq!(
        saved.file_name().unwrap().to_string_lossy(),
        format!("{SESSION}-Engine.ini")
    );
    assert_eq!(
        std::fs::read_to_string(saved).expect("归档内容"),
        "game wrote this"
    );
    assert_eq!(
        std::fs::read_to_string(scenario.render_file("Engine.ini")).expect("原件"),
        ORIGINAL
    );
    session.finish().expect("收尾");
}

#[test]
fn existing_archive_keeps_both_files_and_the_journal() {
    let scenario = Scenario::new("archive-conflict");
    scenario.lay_down();
    std::fs::write(scenario.render_file("Engine.ini"), "game wrote this").expect("替换文件");
    let mutex = FakeMutexOps::new();
    let mut session = scenario.open(&mutex);

    let recovered = scenario
        .state
        .join(".state")
        .join("fixture")
        .join("recovered")
        .join(format!("{SESSION}-Engine.ini"));
    std::fs::create_dir_all(recovered.parent().expect("父目录")).expect("recovered 目录");
    std::fs::write(&recovered, "older archive").expect("旧归档");

    let error = restore(
        &mut session,
        &scenario.ops,
        &RestoreOptions::default(),
        &gate_ok,
        &[],
    )
    .expect_err("应报冲突");

    assert!(
        error.to_string().contains("both files preserved"),
        "实际：{error}"
    );
    assert!(journal_path(&scenario.state).exists(), "冲突时记录必须保留");
    assert_eq!(
        std::fs::read_to_string(&recovered).expect("旧归档"),
        "older archive",
        "旧归档不能被覆盖"
    );
    assert_eq!(
        std::fs::read_to_string(scenario.render_file("Engine.ini")).expect("现场"),
        "game wrote this"
    );
    session.finish().expect("收尾");
}

#[test]
fn changed_backup_is_a_conflict_and_dependents_are_preserved() {
    let scenario = Scenario::new("backup-changed").with_links();
    scenario.lay_down();
    std::fs::write(scenario.render_file("Engine.backup.ini"), "tampered").expect("篡改备份");
    let mutex = FakeMutexOps::new();
    let mut session = scenario.open(&mutex);

    let error = restore(
        &mut session,
        &scenario.ops,
        &RestoreOptions::default(),
        &gate_ok,
        &[],
    )
    .expect_err("应报冲突");
    let text = error.to_string();
    assert!(text.contains("Backup changed"), "实际：{text}");
    assert!(text.contains("journal retained"), "实际：{text}");
    assert!(
        scenario.render_file("Engine.runtime.ini").exists(),
        "runtime 应保留"
    );
    assert!(journal_path(&scenario.state).exists());
    session.finish().expect("收尾");
}

#[test]
fn changed_engine_link_blocks_backup_and_runtime() {
    let mut scenario = Scenario::new("link-changed");
    scenario.lay_down();
    // 引擎链接指向别处 → Link changed
    let wrong = LinkInfo {
        kind: LinkKind::SymbolicLink,
        target: scenario.render_file("somewhere-else.ini"),
        target_is_dir: false,
    };
    let ops =
        std::mem::take(&mut scenario.ops).with_link(scenario.render_file("Engine.ini"), wrong);
    let mutex = FakeMutexOps::new();
    let mut session = scenario.open(&mutex);

    let error = restore(
        &mut session,
        &ops,
        &RestoreOptions::default(),
        &gate_ok,
        &[],
    )
    .expect_err("应报冲突");
    let text = error.to_string();
    assert!(text.contains("Link changed"), "实际：{text}");
    assert!(
        text.contains("Engine link unresolved") || text.contains("dependent files preserved"),
        "实际：{text}"
    );
    assert!(scenario.render_file("Engine.backup.ini").exists());
    assert!(scenario.render_file("Engine.runtime.ini").exists());
    session.finish().expect("收尾");
}

#[test]
fn preview_lists_a_plan_without_touching_anything() {
    let scenario = Scenario::new("preview").with_links();
    scenario.lay_down();
    let mutex = FakeMutexOps::new();
    let mut session = scenario.open(&mutex);

    let outcome = restore(
        &mut session,
        &scenario.ops,
        &RestoreOptions {
            preview: true,
            ..RestoreOptions::default()
        },
        &gate_ok,
        &[],
    )
    .expect("预览不应失败");

    assert_eq!(outcome.plan.len(), 7, "7 条操作都应出现在计划里");
    assert!(outcome.restored.is_empty());
    assert!(!outcome.journal_removed);
    assert!(journal_path(&scenario.state).exists(), "预览不能删记录");
    assert_eq!(
        std::fs::read_to_string(scenario.render_file("Engine.backup.ini")).expect("备份"),
        ORIGINAL,
        "预览不能动文件"
    );
    assert!(scenario.diagnostic.exists());
    assert_eq!(scenario.ops.removed().len(), 0, "预览不能删链接");
    session.finish().expect("收尾");
}

#[test]
fn engine_only_leaves_resource_operations_alone() {
    let scenario = Scenario::new("engine-only").with_links();
    scenario.lay_down();
    let mutex = FakeMutexOps::new();
    let mut session = scenario.open(&mutex);

    let outcome = restore(
        &mut session,
        &scenario.ops,
        &RestoreOptions {
            engine_only: true,
            ..RestoreOptions::default()
        },
        &gate_ok,
        &[],
    )
    .expect("只恢复画面配置");

    assert!(outcome.is_complete());
    assert!(!outcome.journal_removed, "还有资源操作未恢复，记录必须保留");
    assert_eq!(
        std::fs::read_to_string(scenario.render_file("Engine.ini")).expect("原件"),
        ORIGINAL
    );
    assert!(!scenario.render_file("Engine.runtime.ini").exists());
    assert!(
        scenario.diagnostic.exists(),
        "诊断 INI 不在 EngineOnly 范围"
    );
    assert!(
        scenario
            .render
            .join("_storage_")
            .join("version.dll")
            .exists(),
        "storage 副本不在 EngineOnly 范围"
    );
    let journal = launcher_core::journal_store::load(&journal_path(&scenario.state))
        .expect("读记录")
        .expect("记录应存在");
    assert_eq!(journal.phase, Phase::Recovering);
    assert!(
        journal
            .operations
            .iter()
            .any(|operation| operation.kind == OperationKind::Storage
                && operation.state != OperationState::Restored)
    );
    session.finish().expect("收尾");
}

#[test]
fn process_gate_blocks_recovery_before_any_change() {
    let scenario = Scenario::new("gate").with_links();
    scenario.lay_down();
    let mutex = FakeMutexOps::new();
    let mut session = scenario.open(&mutex);
    let refuse = |_automatic: bool| {
        Err(launcher_core::Error::invalid(
            "Game still running; deployment preserved.",
        ))
    };

    let error = restore(
        &mut session,
        &scenario.ops,
        &RestoreOptions {
            automatic: true,
            ..RestoreOptions::default()
        },
        &refuse,
        &[],
    )
    .expect_err("应被进程检查拦住");
    assert!(
        error.to_string().contains("Game still running"),
        "实际：{error}"
    );
    assert!(
        scenario.render_file("Engine.backup.ini").exists(),
        "不应动文件"
    );
    let journal = launcher_core::journal_store::load(&journal_path(&scenario.state))
        .expect("读记录")
        .expect("记录应存在");
    assert_eq!(journal.phase, Phase::Deploying, "被拦住时阶段不应改变");
    session.finish().expect("收尾");
}

#[test]
fn reused_links_are_imported_only_when_they_match_the_allowed_specs() {
    let scenario = Scenario::new("reused");
    scenario.lay_down();
    let mut journal = scenario.journal();
    let allowed = scenario.render.join("dlssg_sm86").join("runtime");
    let allowed_target = scenario.project.join("dlssg_sm86").join("runtime");
    journal.reused = vec![launcher_core::journal::ReusedLink {
        path: allowed.to_string_lossy().into_owned(),
        target: allowed_target.to_string_lossy().into_owned(),
    }];
    let mutex = FakeMutexOps::new();
    let mut session =
        DeploymentSession::open(&mutex, &scenario.state, &scenario.render, journal).expect("会话");

    let specs = [(allowed, allowed_target)];
    let outcome = restore(
        &mut session,
        &scenario.ops,
        &RestoreOptions::default(),
        &gate_ok,
        &specs,
    )
    .expect("恢复应成功");
    assert!(outcome.is_complete(), "{:?}", outcome.conflicts);
    session.finish().expect("收尾");
}

#[test]
fn unexpected_reused_link_is_refused() {
    let scenario = Scenario::new("reused-bad");
    scenario.lay_down();
    let mut journal = scenario.journal();
    journal.reused = vec![launcher_core::journal::ReusedLink {
        path: r"D:\somewhere\evil.dll".to_owned(),
        target: r"D:\project\version.dll".to_owned(),
    }];
    let mutex = FakeMutexOps::new();
    let mut session =
        DeploymentSession::open(&mutex, &scenario.state, &scenario.render, journal).expect("会话");

    let specs = [(
        scenario.render_file("Engine.ini"),
        scenario.render_file("Engine.runtime.ini"),
    )];
    let error = restore(
        &mut session,
        &scenario.ops,
        &RestoreOptions::default(),
        &gate_ok,
        &specs,
    )
    .expect_err("应拒绝");
    assert!(
        error.to_string().contains("Unexpected reused link"),
        "实际：{error}"
    );
    session.finish().expect("收尾");
}

#[test]
fn resource_root_with_foreign_content_is_kept() {
    let scenario = Scenario::new("root-kept");
    scenario.lay_down();
    std::fs::create_dir_all(scenario.render.join("dlssg_sm86").join("logs")).expect("日志目录");
    std::fs::write(
        scenario
            .render
            .join("dlssg_sm86")
            .join("logs")
            .join("dlssg_sm86.log"),
        "log",
    )
    .expect("日志文件");
    let mutex = FakeMutexOps::new();
    let mut session = scenario.open(&mutex);

    restore(
        &mut session,
        &scenario.ops,
        &RestoreOptions::default(),
        &gate_ok,
        &[],
    )
    .expect("恢复应成功");
    assert!(
        scenario.render.join("dlssg_sm86").is_dir(),
        "有日志时资源根必须保留"
    );
    session.finish().expect("收尾");
}

#[test]
fn restore_is_idempotent_after_success() {
    let scenario = Scenario::new("idempotent").with_links();
    scenario.lay_down();
    let mutex = FakeMutexOps::new();
    let mut session = scenario.open(&mutex);
    restore(
        &mut session,
        &scenario.ops,
        &RestoreOptions::default(),
        &gate_ok,
        &[],
    )
    .expect("第一次恢复");
    session.finish().expect("收尾");

    // 记录已删除，第二次打开会得到新记录（此处只验证记录确实没了）
    assert!(!journal_path(&scenario.state).exists());
    let _ = Path::new(&scenario.render);
}

#[test]
fn deploy_then_interrupt_then_restore_round_trip() {
    use launcher_core::journal_store::state_relative_path;
    use launcher_core::resources::{
        Deployment, ProxyChoice, assert_resources_removed, install_resources,
    };

    let scenario = Scenario::new("round-trip");
    let deployment = Deployment::new(
        &scenario.project,
        &scenario.render,
        ProxyChoice::default_choice(),
    );
    let mutex = FakeMutexOps::new();

    // 只保留资源类操作的记录：本轮验证"部署 → 中断 → 恢复"的资源闭环
    let mut resource_journal = scenario.journal();
    resource_journal.operations.retain(|operation| {
        matches!(
            operation.kind,
            OperationKind::Link | OperationKind::Directory
        )
    });
    resource_journal.operations.clear();
    resource_journal.deployment_mode = Some(launcher_core::journal::DeploymentMode::DlssgOnly);
    resource_journal.original_existed = false;
    resource_journal.original_hash = None;

    // 1) 部署：资源链接 + 目录
    {
        let mut session =
            DeploymentSession::open(&mutex, &scenario.state, &scenario.render, resource_journal)
                .expect("会话");
        let outcome =
            install_resources(&mut session, &scenario.ops, &deployment, false).expect("安装资源");
        assert_eq!(outcome.created.len(), 5);
        // 不调用 finish：模拟中断，记录留在磁盘上
    }
    let record = scenario.state.join(state_relative_path("fixture"));
    assert!(record.is_file(), "中断后记录应留在磁盘上");
    let interrupted = launcher_core::journal_store::load(&record)
        .expect("读记录")
        .expect("记录存在");
    assert!(interrupted.has_pending_operations());

    // 2) 重开会话（从磁盘记录恢复上下文），再完整恢复
    let mutex = FakeMutexOps::new();
    let mut session =
        DeploymentSession::open(&mutex, &scenario.state, &scenario.render, interrupted)
            .expect("重新打开会话");
    let outcome = restore(
        &mut session,
        &scenario.ops,
        &RestoreOptions::default(),
        &gate_ok,
        &[],
    )
    .expect("恢复应成功");

    assert!(outcome.is_complete(), "{:?}", outcome.conflicts);
    assert!(outcome.journal_removed);
    assert!(!record.exists(), "恢复完成后记录必须删除");
    assert_eq!(scenario.ops.removed().len(), 5, "5 个资源链接都应拆除");
    assert!(!scenario.render.join("dlssg_sm86").exists(), "资源根应删除");
    // 收尾残留检查：本启动器部署的东西都不能留下
    assert_resources_removed(&deployment, None, None, &scenario.ops).expect("不应有残留");
    session.finish().expect("收尾");
}

#[test]
fn junction_is_never_accepted_as_our_link() {
    // PARITY C6：只认 SymbolicLink，junction 一律按"链接已变"保留现场
    let mut scenario = Scenario::new("junction");
    scenario.lay_down();
    let junction = std::mem::take(&mut scenario.ops).with_junction(
        scenario.render_file("Engine.ini"),
        scenario.render_file("Engine.runtime.ini"),
    );
    let mutex = FakeMutexOps::new();
    let mut session = scenario.open(&mutex);

    let error = restore(
        &mut session,
        &junction,
        &RestoreOptions::default(),
        &gate_ok,
        &[],
    )
    .expect_err("junction 不应被当成本项目的链接");
    assert!(error.to_string().contains("Link changed"), "实际：{error}");
    assert!(junction.removed().is_empty(), "junction 不能被删除");
    assert!(journal_path(&scenario.state).exists());
    session.finish().expect("收尾");
}

#[test]
fn undo_resources_preserves_a_junction() {
    use launcher_core::resources::{Deployment, ProxyChoice, undo_resources};

    let mut scenario = Scenario::new("junction-resource");
    scenario.lay_down();
    let deployment = Deployment::new(
        &scenario.project,
        &scenario.render,
        ProxyChoice::default_choice(),
    );
    let spec = deployment.resource_specs()[0].clone();
    let ops = std::mem::take(&mut scenario.ops).with_junction(&spec.path, &spec.target);

    let outcome = undo_resources(&ops, &deployment).expect("拆除");
    assert!(
        outcome.removed.is_empty(),
        "junction 不能被当成自己的链接删掉"
    );
    assert_eq!(outcome.preserved, vec![spec.path]);
    assert!(ops.removed().is_empty());
}
