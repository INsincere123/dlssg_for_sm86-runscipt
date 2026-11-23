//! 真实软链接的恢复用例（关卡 C 的补充证据）。
//!
//! 默认 `#[ignore]`：创建符号链接需要 `SeCreateSymbolicLinkPrivilege`（管理员或开发者模式）。
//! 在提权终端里跑：
//!
//! ```powershell
//! cd rust
//! cargo test --test real_symlinks -- --ignored --nocapture
//! ```
//!
//! 通过就说明 `restore` 在**真实链接**上同样只删链接本身、并把被替换的 `Engine.ini` 归档。

use std::path::{Path, PathBuf};
use std::time::Duration;

use launcher_core::journal::{Journal, Operation, OperationKind, OperationState};
use launcher_core::journal_store::DeploymentSession;
use launcher_core::ops::{LinkInfo, LinkKind, SymlinkOps};
use launcher_core::restore::{RestoreOptions, restore};
use launcher_core::testing::FakeMutexOps;
use launcher_core::{Error, Result};

const SESSION: &str = "fedcba9876543210fedcba9876543210";

/// 用标准库实现的真实链接操作。
struct RealLinkOps;

impl SymlinkOps for RealLinkOps {
    fn create(&self, link: &Path, target: &Path, target_is_dir: bool) -> Result<()> {
        let result = if target_is_dir {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        };
        result.map_err(|error| Error::io(link, error))
    }

    fn inspect(&self, path: &Path) -> Result<Option<LinkInfo>> {
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(Error::io(path, error)),
        };
        if !metadata.file_type().is_symlink() {
            return Ok(None);
        }
        let target = std::fs::read_link(path).map_err(|error| Error::io(path, error))?;
        // 链接自身的 metadata 永远不是目录：必须看**目标**是不是目录。
        // 目录符号链接要用 RemoveDirectory 删，用 DeleteFile 会失败。
        let target_is_dir = std::fs::metadata(&target)
            .map(|item| item.is_dir())
            .unwrap_or(false);
        Ok(Some(LinkInfo {
            kind: LinkKind::SymbolicLink,
            target,
            target_is_dir,
        }))
    }

    fn remove(&self, path: &Path) -> Result<()> {
        let info = match self.inspect(path)? {
            Some(info) => info,
            None => {
                return Err(Error::invalid(format!(
                    "不是链接，拒绝按链接删除：{}",
                    path.display()
                )));
            }
        };
        let result = if info.target_is_dir {
            std::fs::remove_dir(path)
        } else {
            std::fs::remove_file(path)
        };
        result.map_err(|error| Error::io(path, error))
    }
}

fn scratch(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("dlssg-reallink-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("临时目录");
    directory
}

fn require_links(ops: &RealLinkOps, directory: &Path) -> bool {
    let target = directory.join("probe-target");
    let link = directory.join("probe-link");
    std::fs::write(&target, "probe").expect("探针目标");
    match ops.create(&link, &target, false) {
        Ok(()) => {
            let _ = ops.remove(&link);
            true
        }
        Err(error) => {
            let reason = format!("当前环境无法创建软链接（{error}）");
            // 带 -IncludeIgnored 跑时必须是"真的验证过"：没有权限就失败，不能静默跳过。
            if std::env::var_os("DLSSG_REQUIRE_SYMLINK_TESTS").is_some() {
                panic!("{reason}：请在管理员终端里重跑。");
            }
            eprintln!("跳过：{reason}。请在管理员终端里重跑（check.ps1 -IncludeIgnored）。");
            false
        }
    }
}

#[test]
#[ignore = "需要 SeCreateSymbolicLinkPrivilege：管理员终端里用 --ignored 运行"]
fn real_directory_link_is_removed_without_touching_its_target() {
    let directory = scratch("dir-link");
    let ops = RealLinkOps;
    if !require_links(&ops, &directory) {
        return;
    }
    let project_runtime = directory.join("project").join("dlssg_sm86").join("runtime");
    std::fs::create_dir_all(&project_runtime).expect("项目资源");
    let render = directory.join("render");
    let root = render.join("dlssg_sm86");
    std::fs::create_dir_all(&root).expect("资源根");
    let link = root.join("runtime");
    ops.create(&link, &project_runtime, true)
        .expect("建目录链接");

    let mut journal =
        Journal::from_json(&launcher_core::fixtures::read_text("journal/01-new.json"))
            .expect("样本");
    journal.game_id = "real".to_owned();
    journal.session = SESSION.to_owned();
    journal.render = render.to_string_lossy().into_owned();
    journal.engine = String::new();
    journal.runtime = String::new();
    journal.backup = String::new();
    journal.original_existed = false;
    journal.original_hash = None;
    journal.operations = vec![
        Operation::new(
            OperationKind::Directory,
            root.to_string_lossy().into_owned(),
            OperationState::Done,
        ),
        Operation::new(
            OperationKind::Link,
            link.to_string_lossy().into_owned(),
            OperationState::Done,
        )
        .with_target(project_runtime.to_string_lossy().into_owned()),
    ];

    let mutex = FakeMutexOps::new();
    let state = directory.join("state");
    let mut session = DeploymentSession::open(&mutex, &state, &render, journal).expect("会话");
    let outcome = restore(
        &mut session,
        &ops,
        &RestoreOptions::default(),
        &|_| Ok(()),
        &[],
    )
    .expect("恢复");
    assert!(outcome.is_complete(), "{:?}", outcome.conflicts);
    assert!(!link.exists(), "链接应被删除");
    assert!(
        project_runtime.is_dir(),
        "链接目标必须原样保留（只删链接本身）"
    );
    assert!(!root.exists(), "空的资源根应被删除");
    session.finish().expect("收尾");
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
#[ignore = "需要 SeCreateSymbolicLinkPrivilege：管理员终端里用 --ignored 运行"]
fn replaced_real_link_is_archived_before_restoring_the_original() {
    let directory = scratch("archive-link");
    let ops = RealLinkOps;
    if !require_links(&ops, &directory) {
        return;
    }
    let render = directory.join("render");
    std::fs::create_dir_all(&render).expect("渲染目录");
    let engine = render.join("Engine.ini");
    let runtime = render.join("Engine.runtime.ini");
    let backup = render.join("Engine.backup.ini");
    let original = "original engine.ini\r\n";
    std::fs::write(&backup, original).expect("备份");
    std::fs::write(&runtime, "runtime copy").expect("runtime");
    ops.create(&engine, &runtime, false).expect("建文件链接");
    // 游戏把链接替换成普通文件
    ops.remove(&engine).expect("移除链接");
    std::fs::write(&engine, "game wrote this").expect("替换文件");

    let mut journal =
        Journal::from_json(&launcher_core::fixtures::read_text("journal/01-new.json"))
            .expect("样本");
    journal.game_id = "real".to_owned();
    journal.session = SESSION.to_owned();
    journal.render = render.to_string_lossy().into_owned();
    journal.engine = engine.to_string_lossy().into_owned();
    journal.runtime = runtime.to_string_lossy().into_owned();
    journal.backup = backup.to_string_lossy().into_owned();
    journal.original_existed = true;
    journal.original_hash = Some(launcher_core::path::text_hash_upper(original));
    journal.operations = vec![
        Operation::new(
            OperationKind::Runtime,
            runtime.to_string_lossy().into_owned(),
            OperationState::Done,
        ),
        Operation::new(
            OperationKind::Backup,
            engine.to_string_lossy().into_owned(),
            OperationState::Done,
        )
        .with_target(backup.to_string_lossy().into_owned())
        .with_hash(launcher_core::path::text_hash_upper(original)),
        Operation::new(
            OperationKind::EngineLink,
            engine.to_string_lossy().into_owned(),
            OperationState::Done,
        )
        .with_target(runtime.to_string_lossy().into_owned()),
    ];

    let mutex = FakeMutexOps::new();
    let state = directory.join("state");
    let mut session = DeploymentSession::open(&mutex, &state, &render, journal).expect("会话");
    let outcome = restore(
        &mut session,
        &ops,
        &RestoreOptions::default(),
        &|_| Ok(()),
        &[],
    )
    .expect("恢复");
    assert_eq!(outcome.archived.len(), 1, "应归档被替换的文件");
    assert_eq!(
        std::fs::read_to_string(&engine).expect("原件"),
        original,
        "原件应按字节恢复"
    );
    assert!(
        outcome.archived[0]
            .1
            .file_name()
            .expect("文件名")
            .to_string_lossy()
            .starts_with(SESSION)
    );
    session.finish().expect("收尾");
    let _ = std::fs::remove_dir_all(&directory);
    let _ = Duration::from_secs(0);
}
