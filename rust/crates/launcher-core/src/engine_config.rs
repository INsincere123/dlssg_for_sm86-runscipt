//! `Engine.ini` 的准备（Phase 1 步 8 的"只读副本"，对应 PARITY C8）。
//!
//! 对应现有实现 `Prepare-EngineConfig`（三家 `Run-*.ps1` 各一份）：
//! 1. 原件是链接或目录 → 拒绝（不能把链接备份走）
//! 2. 生成只读运行时副本：`<marker> + CRLF + 配置源原文`，UTF-8 **无 BOM**，属性设为只读
//! 3. 原件存在时先记 `backup` 再移动：`Engine.ini` → `Engine.backup.ini`（按字节，带回哈希）
//! 4. 建 `engineLink`：`Engine.ini` → 运行时副本
//!
//! 每一步都先写 `intent` 再动手、成功后置 `done`；写记录失败必须停止（PARITY C1）。

use std::path::{Path, PathBuf};

use crate::config;
use crate::error::{Error, Result};
use crate::journal::{OperationKind, OperationState};
use crate::journal_store::DeploymentSession;
use crate::ops::SymlinkOps;

/// 运行时副本的注释头。逐字对齐现有实现（PARITY C8）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeMarker {
    Wuthering,
    Neverness,
    /// `; Steam UE5 launcher runtime copy v1 appid=<AppID>`
    Steam {
        app_id: String,
    },
    /// `; Direct UE launcher runtime copy v1 game=<gameId>`
    Direct {
        game_id: String,
    },
}

impl RuntimeMarker {
    pub fn text(&self) -> String {
        match self {
            RuntimeMarker::Wuthering => "; WuWa-Configs launcher runtime copy v1".to_owned(),
            RuntimeMarker::Neverness => "; NTE launcher runtime copy v1".to_owned(),
            RuntimeMarker::Steam { app_id } => {
                format!("; Steam UE5 launcher runtime copy v1 appid={app_id}")
            }
            RuntimeMarker::Direct { game_id } => {
                format!("; Direct UE launcher runtime copy v1 game={game_id}")
            }
        }
    }
}

/// 一次准备的输入。
#[derive(Debug, Clone)]
pub struct EngineConfigSpec {
    /// 游戏的 `Engine.ini` 路径（原件位置，也是最后建链接的位置）。
    pub engine_path: PathBuf,
    /// 运行时副本位置（`%TEMP%` 下的临时文件）。
    pub runtime_path: PathBuf,
    /// 原件备份位置。
    pub backup_path: PathBuf,
    /// 明文配置源（画质配置）。
    pub source_path: PathBuf,
    pub marker: RuntimeMarker,
}

/// 准备结果。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PrepareOutcome {
    pub runtime_written: bool,
    pub backup_moved: bool,
    pub link_created: bool,
    pub link_reused: bool,
    pub original_hash: Option<String>,
}

/// 生成运行时副本的字节：注释头 + CRLF + 源文本，UTF-8 无 BOM。
pub fn runtime_copy_bytes(marker: &RuntimeMarker, source_text: &str) -> Vec<u8> {
    let mut text = String::with_capacity(marker.text().len() + source_text.len() + 2);
    text.push_str(&marker.text());
    text.push_str("\r\n");
    text.push_str(source_text);
    text.into_bytes()
}

/// 读明文配置源并解码（UTF-8 / BOM / UTF-16 / GBK 与 `启动配置.ini` 同一套规则）。
pub fn read_source_text(source_path: &Path) -> Result<String> {
    let bytes = std::fs::read(source_path).map_err(|error| Error::io(source_path, error))?;
    Ok(config::decode(&bytes)?.text)
}

/// 准备 `Engine.ini`：只读副本 → 备份原件 → 建链接。
pub fn prepare_engine_config(
    session: &mut DeploymentSession<'_>,
    ops: &dyn SymlinkOps,
    spec: &EngineConfigSpec,
    dry_run: bool,
) -> Result<PrepareOutcome> {
    if dry_run {
        return Ok(PrepareOutcome::default());
    }
    if session.journal().is_dlssg_only() {
        return Err(Error::invalid("仅 DLSSG 模式禁止画面配置操作。"));
    }

    // 0) 先检查原件：链接或目录一律拒绝，且必须在动手之前（否则会留下半个副本）
    if let Ok(metadata) = std::fs::symlink_metadata(&spec.engine_path) {
        if metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(Error::invalid(format!(
                "原 Engine.ini 是链接或目录，拒绝替换：{}",
                spec.engine_path.display()
            )));
        }
    }

    let source_text = read_source_text(&spec.source_path)?;
    let mut outcome = PrepareOutcome::default();

    // 真实游戏可能还没跑过一次，`Saved\Config\Windows` 不存在；先补目录再建链接。
    if let Some(parent) = spec.engine_path.parent() {
        if !parent.is_dir() {
            std::fs::create_dir_all(parent).map_err(|error| Error::io(parent, error))?;
        }
    }

    // 1) 只读运行时副本
    let runtime_index =
        session.start_operation(OperationKind::Runtime, path_text(&spec.runtime_path)?)?;
    // 上一次准备留下的副本是只读的：同一路径归我们所有，先清属性再覆盖。
    if spec.runtime_path.exists() {
        clear_readonly(&spec.runtime_path)?;
    }
    let bytes = runtime_copy_bytes(&spec.marker, &source_text);
    std::fs::write(&spec.runtime_path, &bytes)
        .map_err(|error| Error::io(&spec.runtime_path, error))?;
    set_readonly(&spec.runtime_path)?;
    session.complete_operation(runtime_index)?;
    outcome.runtime_written = true;

    // 2) 备份原件（不存在就跳过；链接或目录一律拒绝）
    let original = std::fs::symlink_metadata(&spec.engine_path);
    match original {
        Ok(_) => {
            let hash = crate::path::file_hash_or_none(&spec.engine_path)?
                .ok_or_else(|| Error::invalid("原件在准备过程中消失。".to_owned()))?;
            outcome.original_hash = Some(hash.clone());
            let index =
                session.start_operation(OperationKind::Backup, path_text(&spec.engine_path)?)?;
            session.set_operation_target(index, path_text(&spec.backup_path)?)?;
            session.set_operation_hash(index, hash)?;
            std::fs::rename(&spec.engine_path, &spec.backup_path)
                .map_err(|error| Error::io(&spec.engine_path, error))?;
            session.complete_operation(index)?;
            outcome.backup_moved = true;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(Error::io(&spec.engine_path, error)),
    }

    // 3) 引擎链接
    let target = path_text(&spec.runtime_path)?;
    let existing = ops.inspect(&spec.engine_path)?;
    if let Some(link) = existing {
        if link.kind == crate::ops::LinkKind::SymbolicLink
            && same_path(&link.target, Path::new(&target))?
        {
            outcome.link_reused = true;
            return Ok(outcome);
        }
        return Err(Error::invalid(format!(
            "Engine.ini 位置已被占用且不是本次的链接：{}",
            spec.engine_path.display()
        )));
    }
    let index =
        session.start_operation(OperationKind::EngineLink, path_text(&spec.engine_path)?)?;
    session.set_operation_target(index, target)?;
    ops.create(&spec.engine_path, &spec.runtime_path, false)?;
    session.complete_operation(index)?;
    outcome.link_created = true;

    // 记录与现场一致性检查：链接就必须真的指向运行时副本
    debug_assert!(matches!(
        session
            .journal()
            .operations
            .last()
            .map(|operation| operation.state),
        Some(OperationState::Done)
    ));
    Ok(outcome)
}

/// 设置只读属性（现有实现用 `FileAttributes.ReadOnly`）。
pub fn set_readonly(path: &Path) -> Result<()> {
    let mut permissions = std::fs::metadata(path)
        .map_err(|error| Error::io(path, error))?
        .permissions();
    if !permissions.readonly() {
        permissions.set_readonly(true);
        std::fs::set_permissions(path, permissions).map_err(|error| Error::io(path, error))?;
    }
    Ok(())
}

/// 清掉只读属性以便删除或覆盖（恢复流程用）。
// Windows 上只读属性就是删除/覆盖的开关；clippy 的 Unix 语义担忧（world writable）不适用。
#[allow(clippy::permissions_set_readonly_false)]
pub fn clear_readonly(path: &Path) -> Result<()> {
    let mut permissions = std::fs::metadata(path)
        .map_err(|error| Error::io(path, error))?
        .permissions();
    if permissions.readonly() {
        permissions.set_readonly(false);
        std::fs::set_permissions(path, permissions).map_err(|error| Error::io(path, error))?;
    }
    Ok(())
}

fn path_text(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::invalid(format!("路径不是合法 UTF-8：{}", path.display())))
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
    fn markers_match_the_existing_implementations_exactly() {
        assert_eq!(
            RuntimeMarker::Wuthering.text(),
            "; WuWa-Configs launcher runtime copy v1"
        );
        assert_eq!(
            RuntimeMarker::Neverness.text(),
            "; NTE launcher runtime copy v1"
        );
        assert_eq!(
            RuntimeMarker::Steam {
                app_id: "2358720".to_owned()
            }
            .text(),
            "; Steam UE5 launcher runtime copy v1 appid=2358720"
        );
        assert_eq!(
            RuntimeMarker::Direct {
                game_id: "direct-ABCDEF".to_owned()
            }
            .text(),
            "; Direct UE launcher runtime copy v1 game=direct-ABCDEF"
        );
    }

    #[test]
    fn runtime_copy_is_marker_crlf_source_without_bom() {
        let bytes = runtime_copy_bytes(&RuntimeMarker::Neverness, "[A]\r\nKey=1\r\n");
        assert!(
            !bytes.starts_with(&[0xEF, 0xBB, 0xBF]),
            "必须是 UTF-8 无 BOM"
        );
        let text = String::from_utf8(bytes).expect("UTF-8");
        assert_eq!(text, "; NTE launcher runtime copy v1\r\n[A]\r\nKey=1\r\n");
    }
}
