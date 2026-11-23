//! 恢复（Phase 1 步 10–11）。
//!
//! 对照 `Restore-RecordedDeployment`（`DeploymentJournal.ps1:393-498`）与 PARITY C3–C5、C13：
//! - 顺序：`engineLink → backup → runtime → storage → link → diagnostic → directory`，同类逆序
//! - 依赖未完成时**保留依赖文件**（engineLink → backup/runtime；backup → runtime；link → diagnostic）
//! - 单步冲突只记录并继续，最后报 `Recovery incomplete; journal retained: ...`
//! - `Engine.ini` 被游戏替换成普通文件时先归档到 `recovered/<Session>-Engine.ini`
//! - 三种模式：`engine_only`（只碰画面配置）、`preview`（只列计划）、`automatic`（只查游戏进程）
//!
//! 进程检查由调用方注入，core 不猜进程名。

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::journal::{Operation, OperationKind, OperationState, Phase, is_valid_session};
use crate::journal_store::DeploymentSession;
use crate::ops::{LinkKind, SymlinkOps};

/// 恢复开关，对应 PowerShell 的三个 switch。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RestoreOptions {
    /// 只恢复画面配置相关操作（backup/runtime/engineLink）。
    pub engine_only: bool,
    /// 只列计划，不动磁盘、不写记录。
    pub preview: bool,
    /// 正常退出触发的自动恢复：只要求游戏已退出。
    pub automatic: bool,
}

/// 一次恢复的结果。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RestoreOutcome {
    pub restored: Vec<PathBuf>,
    pub archived: Vec<(PathBuf, PathBuf)>,
    pub conflicts: Vec<String>,
    pub plan: Vec<String>,
    pub journal_removed: bool,
}

impl RestoreOutcome {
    pub fn is_complete(&self) -> bool {
        self.conflicts.is_empty()
    }
}

/// 进程检查：`automatic` 表示正常退出的自动恢复（宽松），否则要求游戏与启动器都已退出。
pub type ProcessGate<'a> = dyn Fn(bool) -> Result<()> + 'a;

/// 恢复记录里的部署。
///
/// `allowed_links` 用于校验旧记录里的 `Reused` 链接（PS 的 `Assert-DeploymentLinkSpec`）：
/// 只有落在这些规格里的链接才会被纳入恢复。传空切片表示跳过 `Reused` 导入。
pub fn restore(
    session: &mut DeploymentSession<'_>,
    ops: &dyn SymlinkOps,
    options: &RestoreOptions,
    ensure_stopped: &ProcessGate<'_>,
    allowed_links: &[(PathBuf, PathBuf)],
) -> Result<RestoreOutcome> {
    session.journal().validate()?;
    let mut outcome = RestoreOutcome::default();

    if !options.preview {
        ensure_stopped(options.automatic)?;
        session.set_phase(Phase::Recovering)?;
        if !options.engine_only {
            import_reused_links(session, allowed_links)?;
        }
    }

    let order = session.journal().restore_order_indices();
    for index in order {
        let operation = session.journal().operations[index].clone();
        if operation.state == OperationState::Restored {
            continue;
        }
        if options.engine_only
            && !matches!(
                operation.kind,
                OperationKind::Backup | OperationKind::Runtime | OperationKind::EngineLink
            )
        {
            continue;
        }
        if options.preview {
            outcome.plan.push(format!(
                "Recovery: {} {}",
                operation.kind.as_str(),
                operation.path
            ));
            continue;
        }
        match restore_one(session, ops, index, &operation) {
            Ok(action) => {
                match action {
                    Action::Restored => outcome.restored.push(PathBuf::from(&operation.path)),
                    Action::Archived(saved) => {
                        outcome
                            .archived
                            .push((PathBuf::from(&operation.path), saved.clone()));
                        outcome.restored.push(PathBuf::from(&operation.path));
                    }
                    Action::AlreadyGone => {}
                }
                session.mark_restored(index)?;
            }
            Err(conflict) => outcome.conflicts.push(conflict.to_string()),
        }
    }

    if !outcome.conflicts.is_empty() {
        return Err(Error::invalid(format!(
            "Recovery incomplete; journal retained: {}",
            outcome.conflicts.join("; ")
        )));
    }
    if !options.preview && !options.engine_only {
        session.remove_journal()?;
        outcome.journal_removed = true;
    }
    Ok(outcome)
}

enum Action {
    Restored,
    Archived(PathBuf),
    AlreadyGone,
}

fn restore_one(
    session: &mut DeploymentSession<'_>,
    ops: &dyn SymlinkOps,
    index: usize,
    operation: &Operation,
) -> Result<Action> {
    check_dependencies(session, index, operation)?;
    let session_id = session.journal().session.clone();
    match operation.kind {
        OperationKind::EngineLink => restore_engine_link(session, ops, operation, &session_id),
        OperationKind::Backup => restore_backup(operation),
        OperationKind::Runtime => remove_temporary(operation, true),
        OperationKind::Diagnostic | OperationKind::Storage => remove_temporary(operation, false),
        OperationKind::Link => remove_link(ops, operation),
        OperationKind::Directory => remove_directory(operation),
    }
}

/// 依赖未完成时保留依赖文件（PARITY C4）。
fn check_dependencies(
    session: &DeploymentSession<'_>,
    index: usize,
    operation: &Operation,
) -> Result<()> {
    let pending = |kind: OperationKind, skip: usize| -> Vec<usize> {
        session
            .journal()
            .operations
            .iter()
            .enumerate()
            .filter(|(position, candidate)| {
                *position != skip
                    && candidate.kind == kind
                    && candidate.state != OperationState::Restored
            })
            .map(|(position, _)| position)
            .collect()
    };

    if matches!(
        operation.kind,
        OperationKind::Backup | OperationKind::Runtime
    ) && !pending(OperationKind::EngineLink, index).is_empty()
    {
        return Err(Error::invalid(
            "Engine link unresolved; dependent files preserved.",
        ));
    }
    if operation.kind == OperationKind::Runtime && !pending(OperationKind::Backup, index).is_empty()
    {
        return Err(Error::invalid("Backup unresolved; runtime preserved."));
    }
    if operation.kind == OperationKind::Diagnostic {
        let target = normalized(&operation.path)?;
        let blocking =
            session
                .journal()
                .operations
                .iter()
                .enumerate()
                .any(|(position, candidate)| {
                    position != index
                        && candidate.kind == OperationKind::Link
                        && candidate.state != OperationState::Restored
                        && normalized(&candidate.target)
                            .map(|value| value == target)
                            .unwrap_or(false)
                });
        if blocking {
            return Err(Error::invalid(
                "Diagnostic link unresolved; target preserved.",
            ));
        }
    }
    Ok(())
}

fn restore_engine_link(
    session: &DeploymentSession<'_>,
    ops: &dyn SymlinkOps,
    operation: &Operation,
    session_id: &str,
) -> Result<Action> {
    let backup = session
        .journal()
        .operations
        .iter()
        .find(|candidate| candidate.kind == OperationKind::Backup)
        .cloned();

    let path = Path::new(&operation.path);
    if let Some(backup) = &backup {
        let backup_intact = is_ordinary_file_with_hash(Path::new(&backup.target), &backup.hash)?;
        if !backup_intact && exists(path) {
            return Err(Error::invalid(
                "Backup missing/changed; Engine.ini preserved.",
            ));
        }
    }

    // 游戏可能把只读软链接替换成普通文件：先归档，再恢复原件。
    if exists(path) {
        let metadata = std::fs::symlink_metadata(path).map_err(|error| Error::io(path, error))?;
        if operation.state == OperationState::Done
            && !metadata.is_dir()
            && !metadata.file_type().is_symlink()
        {
            if !is_valid_session(session_id) {
                return Err(Error::invalid("Invalid recovery session."));
            }
            let recovered_directory = session
                .path()
                .parent()
                .expect("记录一定有父目录")
                .join("recovered");
            let saved = recovered_directory.join(format!("{session_id}-Engine.ini"));
            if exists(&saved) {
                return Err(Error::invalid(format!(
                    "Replacement backup already exists; both files preserved: {}",
                    saved.display()
                )));
            }
            std::fs::create_dir_all(&recovered_directory)
                .map_err(|error| Error::io(&recovered_directory, error))?;
            std::fs::rename(&operation.path, &saved)
                .map_err(|error| Error::io(&operation.path, error))?;
            return Ok(Action::Archived(saved));
        }
    }

    let info = ops.inspect(path)?;
    if !exists(path) && info.is_none() {
        return Ok(Action::AlreadyGone);
    }
    match info {
        Some(link)
            if link.kind == LinkKind::SymbolicLink
                && same_path(&link.target, Path::new(&operation.target))? =>
        {
            ops.remove(path)?;
            Ok(Action::Restored)
        }
        _ => Err(Error::invalid(format!(
            "Link changed; preserved: {}",
            operation.path
        ))),
    }
}

fn restore_backup(operation: &Operation) -> Result<Action> {
    let backup = Path::new(&operation.target);
    let original = Path::new(&operation.path);
    if exists(backup) {
        if !is_ordinary_file_with_hash(backup, &operation.hash)? {
            return Err(Error::invalid(format!(
                "Backup changed; preserved: {}",
                operation.target
            )));
        }
        if exists(original) {
            let intact = is_ordinary_file_with_hash(original, &operation.hash)?;
            return if intact {
                Err(Error::invalid(format!(
                    "Both original and backup exist; preserved: {}",
                    operation.path
                )))
            } else {
                Err(Error::invalid(format!(
                    "Original path occupied: {}",
                    operation.path
                )))
            };
        }
        std::fs::rename(backup, original).map_err(|error| Error::io(backup, error))?;
        return Ok(Action::Restored);
    }
    if is_ordinary_file_with_hash(original, &operation.hash)? {
        Ok(Action::AlreadyGone)
    } else {
        Err(Error::invalid(format!(
            "Original/backup missing or changed: {}",
            operation.path
        )))
    }
}

/// `runtime` 允许游戏改写内容，因此不校验哈希；`diagnostic`/`storage` 必须哈希一致。
fn remove_temporary(operation: &Operation, allow_rewrite: bool) -> Result<Action> {
    let path = Path::new(&operation.path);
    if !exists(path) {
        return Ok(Action::AlreadyGone);
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|error| Error::io(path, error))?;
    if metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(Error::invalid(format!(
            "Temporary file replaced; preserved: {}",
            operation.path
        )));
    }
    if !allow_rewrite && crate::path::file_hash_or_none(path)? != Some(operation.hash.clone()) {
        return Err(Error::invalid(format!(
            "File hash changed; preserved: {}",
            operation.path
        )));
    }
    // 运行时副本是只读的：现有实现用 `Remove-Item -Force`，这里先清属性再删。
    crate::engine_config::clear_readonly(path)?;
    std::fs::remove_file(path).map_err(|error| Error::io(path, error))?;
    Ok(Action::Restored)
}

fn remove_link(ops: &dyn SymlinkOps, operation: &Operation) -> Result<Action> {
    let path = Path::new(&operation.path);
    let info = ops.inspect(path)?;
    if !exists(path) && info.is_none() {
        return Ok(Action::AlreadyGone);
    }
    match info {
        Some(link)
            if link.kind == LinkKind::SymbolicLink
                && same_path(&link.target, Path::new(&operation.target))? =>
        {
            ops.remove(path)?;
            Ok(Action::Restored)
        }
        _ => Err(Error::invalid(format!(
            "Link changed; preserved: {}",
            operation.path
        ))),
    }
}

fn remove_directory(operation: &Operation) -> Result<Action> {
    let path = Path::new(&operation.path);
    if !exists(path) {
        return Ok(Action::AlreadyGone);
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|error| Error::io(path, error))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(Error::invalid(format!(
            "Resource directory changed; preserved: {}",
            operation.path
        )));
    }
    // 只删空目录：游戏日志等无关内容必须保留，绝不递归删除。
    let mut entries = std::fs::read_dir(path).map_err(|error| Error::io(path, error))?;
    if entries.next().is_none() {
        std::fs::remove_dir(path).map_err(|error| Error::io(path, error))?;
    }
    Ok(Action::Restored)
}

fn import_reused_links(
    session: &mut DeploymentSession<'_>,
    allowed_links: &[(PathBuf, PathBuf)],
) -> Result<()> {
    if allowed_links.is_empty() {
        return Ok(());
    }
    let reused = session.journal().reused.clone();
    for link in reused {
        let allowed = allowed_links.iter().any(|(path, target)| {
            same_path(path, Path::new(&link.path)).unwrap_or(false)
                && same_path(target, Path::new(&link.target)).unwrap_or(false)
        });
        if !allowed {
            return Err(Error::invalid(format!(
                "Unexpected reused link: {} -> {}",
                link.path, link.target
            )));
        }
        let already = session.journal().operations.iter().any(|operation| {
            operation.kind == OperationKind::Link && operation.path.eq_ignore_ascii_case(&link.path)
        });
        if already {
            continue;
        }
        let index = session.start_operation(OperationKind::Link, link.path)?;
        session.set_operation_target(index, link.target)?;
        session.complete_operation(index)?;
    }
    Ok(())
}

fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

fn is_ordinary_file_with_hash(path: &Path, expected: &str) -> Result<bool> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return Ok(false),
    };
    if metadata.is_dir() || metadata.file_type().is_symlink() {
        return Ok(false);
    }
    Ok(crate::path::file_hash_or_none(path)? == Some(expected.to_owned()))
}

fn normalized(path: &str) -> Result<String> {
    crate::path::PathContext::current()?.normalize(path)
}

fn same_path(left: &Path, right: &Path) -> Result<bool> {
    let context = crate::path::PathContext::current()?;
    Ok(context.normalize(&left.to_string_lossy())?
        == context.normalize(&right.to_string_lossy())?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_options_are_a_full_restore() {
        let options = RestoreOptions::default();
        assert!(!options.engine_only);
        assert!(!options.preview);
        assert!(!options.automatic);
    }
}
