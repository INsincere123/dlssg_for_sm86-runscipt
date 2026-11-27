//! Phase 1 步 8–9：资源规格、断言、安装与拆除。
//!
//! 对照 `LauncherResources.ps1:84-165`。链接能力由 `FakeSymlinkOps` 提供，
//! 因此不需要管理员权限；普通文件与目录仍用真实临时目录。

use std::path::{Path, PathBuf};

use launcher_core::journal::Journal;
use launcher_core::journal_store::DeploymentSession;
use launcher_core::ops::{LinkInfo, LinkKind};
use launcher_core::resources::{
    CORE_REQUIRED, Deployment, LinkOutcome, ProxyChoice, StorageClaim, assert_deployment,
    assert_required_files, assert_resources_removed, claim_storage, install_proxy_link,
    install_resources, missing_super_resolution_files, required_files, scan_other_proxies,
    super_resolution_required, undo_resources,
};
use launcher_core::testing::{FakeMutexOps, FakeSymlinkOps};
use launcher_core::{Error, SymlinkOps};

struct Fixture {
    root: PathBuf,
    project: PathBuf,
    render: PathBuf,
}

fn setup(name: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("dlssg-step0809-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("project");
    let render = root.join("render");
    std::fs::create_dir_all(&project).expect("项目目录");
    std::fs::create_dir_all(&render).expect("渲染目录");
    for relative in CORE_REQUIRED {
        let path = project.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("资源目录");
        }
        std::fs::write(&path, "resource").expect("资源文件");
    }
    std::fs::write(project.join("version.dll"), "proxy-payload").expect("代理文件");
    // 全量包才有的 sr 目录：这些用例按全量布局跑，轻量版的行为由单元测试覆盖。
    std::fs::create_dir_all(project.join("dlssg_sm86").join("sr")).expect("sr 目录");
    for name in ["sr_kernels.dll", "nvngx_dlss.dll"] {
        std::fs::write(project.join("dlssg_sm86").join("sr").join(name), "resource")
            .expect("sr 资源");
    }
    let alternates = project.join("alternatives");
    std::fs::create_dir_all(&alternates).expect("alternatives");
    for name in [
        "winmm.dll",
        "dbghelp.dll",
        "dinput8.dll",
        "dxgi.dll",
        "d3d12.dll",
    ] {
        std::fs::write(alternates.join(name), format!("proxy-{name}")).expect("备选代理");
    }
    Fixture {
        root,
        project,
        render,
    }
}

impl Fixture {
    fn deployment(&self, index: &str) -> Deployment<'_> {
        Deployment::new(
            &self.project,
            &self.render,
            ProxyChoice::from_index(index).expect("代理编号"),
        )
    }

    fn session_journal(&self) -> Journal {
        let mut journal =
            Journal::from_json(&launcher_core::fixtures::read_text("journal/01-new.json"))
                .expect("样本");
        journal.session = "0123456789abcdef0123456789abcdef".to_owned();
        journal.game_id = "fixture".to_owned();
        journal
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn fake_link(target: &Path, target_is_dir: bool) -> LinkInfo {
    LinkInfo {
        kind: LinkKind::SymbolicLink,
        target: target.to_path_buf(),
        target_is_dir,
    }
}

#[test]
fn required_resources_depend_on_super_resolution() {
    let fixture = setup("required");
    // 先按轻量版的样子把 sr 内核删掉：核心资源仍然齐全，只有开超分才该报错。
    for relative in super_resolution_required() {
        std::fs::remove_file(fixture.project.join(relative)).expect("删掉 sr 内核");
    }
    assert_eq!(required_files(false).len(), 5);
    assert_required_files(&fixture.project, false).expect("核心资源齐全");

    let error = assert_required_files(&fixture.project, true).expect_err("缺 sr 应报错");
    let text = error.to_string();
    assert!(text.contains("Missing DLSSG resource"), "实际：{text}");
    assert!(text.contains("sr_kernels.dll"), "实际：{text}");

    for relative in super_resolution_required() {
        let path = fixture.project.join(relative);
        std::fs::create_dir_all(path.parent().expect("父目录")).expect("sr 目录");
        std::fs::write(&path, "sr").expect("sr 文件");
    }
    assert_required_files(&fixture.project, true).expect("补齐后通过");
    for relative in super_resolution_required() {
        let path = fixture.project.join(relative);
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert_eq!(
            missing_super_resolution_files(&fixture.project),
            std::slice::from_ref(&path)
        );
        assert_required_files(&fixture.project, false).unwrap();
        let error = assert_required_files(&fixture.project, true).unwrap_err();
        assert!(
            error.to_string().contains(&*path.to_string_lossy()),
            "{error}"
        );
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, "sr").unwrap();
    }
    assert!(missing_super_resolution_files(&fixture.project).is_empty());
}

#[test]
fn missing_core_resource_is_named() {
    let fixture = setup("missing-core");
    std::fs::remove_file(fixture.project.join(CORE_REQUIRED[1])).expect("删掉后端");
    let error = assert_required_files(&fixture.project, false).expect_err("应报错");
    assert!(
        error.to_string().contains("nvngx_dlssg.dll"),
        "实际：{error}"
    );
}

#[test]
fn foreign_proxy_files_are_reported_but_not_fatal() {
    let fixture = setup("foreign");
    let ops = FakeSymlinkOps::new();
    let foreign = fixture.render.join("dbghelp.dll");
    std::fs::write(&foreign, "third-party injector").expect("第三方文件");

    let deployment = fixture.deployment("1");
    let scan = assert_deployment(&deployment, false, &ops).expect("第三方文件不应阻止部署");
    assert_eq!(scan.foreign, vec![foreign]);
    assert!(scan.stale.is_empty());
}

#[test]
fn stale_links_into_the_project_must_be_restored_first() {
    let fixture = setup("stale");
    let deployment = fixture.deployment("1");
    let stale = fixture.render.join("winmm.dll");
    let ops = FakeSymlinkOps::new().with_link(
        &stale,
        fake_link(&fixture.project.join("alternatives\\winmm.dll"), false),
    );

    let error = assert_deployment(&deployment, false, &ops).expect_err("应拒绝");
    assert!(
        error
            .to_string()
            .contains("Stale deployment link must be restored first"),
        "实际：{error}"
    );

    let scan = scan_other_proxies(&deployment, &ops).expect("扫描");
    assert_eq!(scan.stale, vec![stale]);
}

#[test]
fn resource_root_must_be_an_ordinary_directory() {
    let fixture = setup("root-file");
    let ops = FakeSymlinkOps::new();
    std::fs::write(fixture.render.join("dlssg_sm86"), "not a directory").expect("占位文件");
    let deployment = fixture.deployment("1");
    let error = assert_deployment(&deployment, false, &ops).expect_err("应拒绝");
    assert!(
        error
            .to_string()
            .contains("Resource root must be an ordinary directory"),
        "实际：{error}"
    );
}

#[test]
fn resource_path_conflict_is_refused() {
    let fixture = setup("conflict");
    let ops = FakeSymlinkOps::new();
    // 普通目录（不是链接）占住了 runtime 位置
    std::fs::create_dir_all(fixture.render.join("dlssg_sm86").join("runtime")).expect("占位目录");
    let deployment = fixture.deployment("1");
    let error = assert_deployment(&deployment, false, &ops).expect_err("应拒绝");
    assert!(
        error.to_string().contains("Resource path conflict"),
        "实际：{error}"
    );
    assert!(
        fixture.render.join("dlssg_sm86").join("runtime").is_dir(),
        "冲突现场应保留"
    );
}

#[test]
fn matching_links_are_accepted_for_reuse() {
    let fixture = setup("reuse");
    let mut ops = FakeSymlinkOps::new();
    let deployment = fixture.deployment("1");
    std::fs::create_dir_all(fixture.render.join("dlssg_sm86")).expect("资源根");
    for spec in deployment.resource_specs() {
        std::fs::create_dir_all(&spec.path).expect("占位目录");
        ops = ops.with_link(&spec.path, fake_link(&spec.target, true));
    }
    let scan = assert_deployment(&deployment, false, &ops).expect("复用应通过");
    assert!(scan.stale.is_empty() && scan.foreign.is_empty());
}

#[test]
fn install_creates_the_root_and_five_links() {
    let fixture = setup("install");
    let ops_symlink = FakeSymlinkOps::new();
    let ops_mutex = FakeMutexOps::new();
    let deployment = fixture.deployment("1");
    let mut session = DeploymentSession::open(
        &ops_mutex,
        &fixture.root,
        &fixture.render,
        fixture.session_journal(),
    )
    .expect("会话");

    let outcome = install_resources(&mut session, &ops_symlink, &deployment, false).expect("安装");
    assert_eq!(outcome.created.len(), 5);
    assert!(outcome.reused.is_empty());
    assert_eq!(outcome.directory_created, Some(deployment.resource_root()));
    assert!(deployment.resource_root().is_dir(), "资源根应被创建");

    let journal = session.journal();
    assert_eq!(
        journal
            .operations_of(launcher_core::journal::OperationKind::Directory)
            .len(),
        1
    );
    let links = journal.operations_of(launcher_core::journal::OperationKind::Link);
    assert_eq!(links.len(), 5);
    assert!(links.iter().all(|operation| {
        operation.state == launcher_core::journal::OperationState::Done
            && !operation.target.is_empty()
    }));
    for spec in deployment.resource_specs() {
        let info = ops_symlink
            .inspect(&spec.path)
            .expect("查询")
            .expect("应有链接");
        assert_eq!(info.target, spec.target);
        assert!(info.target_is_dir);
    }
    session.finish().expect("收尾");
}

#[test]
fn install_reuses_existing_links_without_duplicating_records() {
    let fixture = setup("install-reuse");
    let ops_symlink = FakeSymlinkOps::new();
    let ops_mutex = FakeMutexOps::new();
    let deployment = fixture.deployment("1");
    let mut session = DeploymentSession::open(
        &ops_mutex,
        &fixture.root,
        &fixture.render,
        fixture.session_journal(),
    )
    .expect("会话");

    install_resources(&mut session, &ops_symlink, &deployment, false).expect("首次安装");
    let outcome =
        install_resources(&mut session, &ops_symlink, &deployment, false).expect("再次安装");
    assert!(outcome.created.is_empty(), "不应重复创建");
    assert_eq!(outcome.reused.len(), 5);
    let links = session
        .journal()
        .operations_of(launcher_core::journal::OperationKind::Link);
    assert_eq!(links.len(), 5, "复用不应追加重复记录");
    session.finish().expect("收尾");
}

#[test]
fn dry_run_touches_neither_disk_nor_journal() {
    let fixture = setup("dry-run");
    let ops_symlink = FakeSymlinkOps::new();
    let ops_mutex = FakeMutexOps::new();
    let deployment = fixture.deployment("1");
    let mut session = DeploymentSession::open(
        &ops_mutex,
        &fixture.root,
        &fixture.render,
        fixture.session_journal(),
    )
    .expect("会话");

    let outcome = install_resources(&mut session, &ops_symlink, &deployment, true).expect("演练");
    assert_eq!(outcome.created.len(), 5);
    assert!(!deployment.resource_root().exists(), "演练不应建目录");
    assert!(session.journal().operations.is_empty(), "演练不应写操作");
    assert_eq!(ops_symlink.count(), 0, "演练不应建链接");
    session.finish().expect("收尾");
}

#[test]
fn undo_removes_only_our_links_and_drops_the_empty_root() {
    let fixture = setup("undo");
    let ops_symlink = FakeSymlinkOps::new();
    let ops_mutex = FakeMutexOps::new();
    let deployment = fixture.deployment("1");
    let mut session = DeploymentSession::open(
        &ops_mutex,
        &fixture.root,
        &fixture.render,
        fixture.session_journal(),
    )
    .expect("会话");
    install_resources(&mut session, &ops_symlink, &deployment, false).expect("安装");

    // 链接被替换成普通目录：链接层不再认为自己有这条链接，恢复时必须保留现场
    let preserved = deployment.resource_root().join("runtime");
    ops_symlink.remove(&preserved).expect("链接层移除");
    let _ = std::fs::remove_dir_all(&preserved);
    std::fs::create_dir_all(&preserved).expect("占位目录");

    let outcome = undo_resources(&ops_symlink, &deployment).expect("拆除");
    assert_eq!(outcome.removed.len(), 4, "只删我们建的四个链接");
    assert_eq!(outcome.preserved, vec![preserved.clone()]);
    assert!(preserved.is_dir(), "第三方目录应保留");
    assert!(!outcome.root_removed, "还有内容时资源根不能删");
    session.finish().expect("收尾");

    std::fs::remove_dir_all(&preserved).expect("清理占位");
    let outcome = undo_resources(&ops_symlink, &deployment).expect("再拆除");
    assert!(outcome.removed.is_empty());
    assert!(outcome.root_removed || !deployment.resource_root().exists());
}

#[test]
fn storage_copy_is_claimed_only_when_the_hash_matches() {
    let fixture = setup("storage");
    let deployment = fixture.deployment("1");
    assert_eq!(
        claim_storage(&deployment).expect("认领"),
        StorageClaim::Absent
    );

    let storage = deployment.storage_dll();
    std::fs::create_dir_all(storage.parent().expect("父目录")).expect("_storage_");
    std::fs::copy(deployment.proxy_target(), &storage).expect("复制");
    match claim_storage(&deployment).expect("认领") {
        StorageClaim::Owned(hash) => assert_eq!(hash.len(), 64),
        other => panic!("应认领为 Owned，实际 {other:?}"),
    }

    std::fs::write(&storage, "different payload").expect("改成不同内容");
    match claim_storage(&deployment).expect("认领") {
        StorageClaim::Foreign(hash) => assert_eq!(hash.len(), 64),
        other => panic!("应判为 Foreign，实际 {other:?}"),
    }
}

#[test]
fn invalid_proxy_index_is_refused_before_any_io() {
    let error = ProxyChoice::from_index("9").expect_err("应拒绝");
    assert!(matches!(error, Error::Invalid { .. }));
    assert!(error.to_string().contains("超出范围"), "实际：{error}");
}

#[test]
fn residue_check_reports_links_runtime_and_owned_storage() {
    let fixture = setup("residue");
    let ops = FakeSymlinkOps::new();
    let deployment = fixture.deployment("1");

    // 干净状态：通过
    assert_resources_removed(&deployment, None, None, &ops).expect("干净状态应通过");

    // 有我们建的链接 → 报残留
    let spec = deployment.resource_specs()[0].clone();
    let ops_with_link = FakeSymlinkOps::new().with_link(&spec.path, fake_link(&spec.target, true));
    let error =
        assert_resources_removed(&deployment, None, None, &ops_with_link).expect_err("应有残留");
    assert!(
        error.to_string().contains("Deployment residue preserved"),
        "实际：{error}"
    );

    // 运行时副本/备份还在 → 报残留
    let runtime = fixture.root.join("runtime.ini");
    std::fs::write(&runtime, "x").expect("运行时副本");
    assert!(assert_resources_removed(&deployment, Some(&runtime), None, &ops).is_err());

    // _storage_ 与项目代理一致 → 报"我们的残留"
    let storage = deployment.storage_dll();
    std::fs::create_dir_all(storage.parent().expect("父目录")).expect("_storage_");
    std::fs::copy(deployment.proxy_target(), &storage).expect("复制代理");
    let error = assert_resources_removed(&deployment, None, None, &ops).expect_err("应报残留");
    assert!(
        error.to_string().contains("Owned storage DLL"),
        "实际：{error}"
    );

    // 哈希不同 → 属于别人，不算残留
    std::fs::write(&storage, "foreign payload").expect("改成别人的内容");
    assert_resources_removed(&deployment, None, None, &ops).expect("外来副本不应报残留");
}

#[test]
fn proxy_link_is_created_then_reused() {
    let fixture = setup("proxy");
    let ops = FakeSymlinkOps::new();
    let mutex = FakeMutexOps::new();
    let deployment = fixture.deployment("2");
    let mut session = DeploymentSession::open(
        &mutex,
        &fixture.root,
        &fixture.render,
        fixture.session_journal(),
    )
    .expect("会话");

    assert_eq!(
        install_proxy_link(&mut session, &ops, &deployment, false).expect("部署代理"),
        LinkOutcome::Created
    );
    let info = ops
        .inspect(&deployment.proxy_path())
        .expect("查询")
        .expect("应有代理链接");
    assert_eq!(info.target, deployment.proxy_target());
    assert!(!info.target_is_dir);

    assert_eq!(
        install_proxy_link(&mut session, &ops, &deployment, false).expect("复用代理"),
        LinkOutcome::Reused
    );
    assert_eq!(
        session
            .journal()
            .operations_of(launcher_core::journal::OperationKind::Link)
            .len(),
        1,
        "复用不应追加记录"
    );
    session.finish().expect("收尾");
}

#[test]
fn third_party_proxy_file_is_never_overwritten() {
    let fixture = setup("proxy-foreign");
    let ops = FakeSymlinkOps::new();
    let mutex = FakeMutexOps::new();
    let deployment = fixture.deployment("1");
    std::fs::write(deployment.proxy_path(), "someone else").expect("第三方代理");
    let mut session = DeploymentSession::open(
        &mutex,
        &fixture.root,
        &fixture.render,
        fixture.session_journal(),
    )
    .expect("会话");

    let error = install_proxy_link(&mut session, &ops, &deployment, false).expect_err("应拒绝");
    assert!(error.to_string().contains("第三方代理"), "实际：{error}");
    assert_eq!(
        std::fs::read_to_string(deployment.proxy_path()).expect("原文件"),
        "someone else"
    );
    session.finish().expect("收尾");
}
