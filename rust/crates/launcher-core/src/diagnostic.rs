//! 临时诊断 INI 的部署（Phase 1 步 8 的最后一块，PARITY C7）。
//!
//! 对应现有实现里 `$linkSpecs[1]` 那一半：
//! 1.  `$diagnosticIniPath = <TEMP>\dlssg_sm86-<game>-<pid>.ini`，已存在则拒绝覆盖
//! 2.  记一条 `diagnostic` 操作（哈希是**文本哈希**，不是文件哈希），再写入文本
//! 3.  把 `<渲染目录>\dlssg_sm86.ini` 链到它，记一条 `link` 操作
//!
//! 恢复流程已经有对应的拆除逻辑（`restore` 的 `diagnostic` / `link` 分支），
//! 这一步补的是"部署侧"。

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::journal::{OperationKind, OperationState};
use crate::journal_store::DeploymentSession;
use crate::ops::SymlinkOps;

/// 一次诊断 INI 部署的输入。
#[derive(Debug, Clone)]
pub struct DiagnosticSpec {
    /// 渲染目录，链接落点是 `<render_dir>\dlssg_sm86.ini`。
    pub render_dir: PathBuf,
    /// `%TEMP%` 下的临时文件位置。
    pub temp_path: PathBuf,
    /// 已生成好的诊断文本（由 `settings::render_diagnostic` 产出）。
    pub text: String,
}

impl DiagnosticSpec {
    /// 链接位置：`<渲染目录>\dlssg_sm86.ini`。
    pub fn link_path(&self) -> PathBuf {
        self.render_dir.join("dlssg_sm86.ini")
    }
}

/// 部署结果。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DiagnosticOutcome {
    pub temp_written: bool,
    pub link_created: bool,
    pub link_reused: bool,
    pub text_hash: Option<String>,
}

/// 部署诊断 INI 与它的链接。
pub fn deploy_diagnostic(
    session: &mut DeploymentSession<'_>,
    ops: &dyn SymlinkOps,
    spec: &DiagnosticSpec,
    dry_run: bool,
) -> Result<DiagnosticOutcome> {
    if dry_run {
        return Ok(DiagnosticOutcome::default());
    }
    if session.journal().is_dlssg_only() {
        // 仅 DLSSG 模式仍然需要诊断 INI 与代理一起工作，所以这里不拒绝；
        // 但画面配置操作（backup/runtime/engineLink）由 journal 层拒绝。
    }

    // 1) 临时文件：存在即拒绝（现有实现同样拒绝覆盖）
    if std::fs::symlink_metadata(&spec.temp_path).is_ok() {
        return Err(Error::invalid(format!(
            "临时诊断配置路径已存在，拒绝覆盖：{}",
            spec.temp_path.display()
        )));
    }
    let text_hash = crate::path::text_hash_upper(&spec.text);
    let index = session.start_operation(OperationKind::Diagnostic, path_text(&spec.temp_path)?)?;
    session.set_operation_hash(index, text_hash.clone())?;
    std::fs::write(&spec.temp_path, spec.text.as_bytes())
        .map_err(|error| Error::io(&spec.temp_path, error))?;
    session.complete_operation(index)?;

    // 2) 链接：已指向同一临时文件就复用
    let link_path = spec.link_path();
    if let Some(info) = ops.inspect(&link_path)? {
        let matches = info.kind == crate::ops::LinkKind::SymbolicLink
            && same_path(&info.target, &spec.temp_path)?;
        if matches {
            return Ok(DiagnosticOutcome {
                temp_written: true,
                link_created: false,
                link_reused: true,
                text_hash: Some(text_hash),
            });
        }
        return Err(Error::invalid(format!(
            "dlssg_sm86.ini 位置已被占用且不是本次的链接：{}",
            link_path.display()
        )));
    }
    if std::fs::symlink_metadata(&link_path).is_ok() {
        return Err(Error::invalid(format!(
            "dlssg_sm86.ini 位置已有普通文件，拒绝覆盖：{}",
            link_path.display()
        )));
    }

    let target = path_text(&spec.temp_path)?;
    let index = session.start_operation(OperationKind::Link, path_text(&link_path)?)?;
    session.set_operation_target(index, target)?;
    ops.create(&link_path, &spec.temp_path, false)?;
    session.complete_operation(index)?;

    debug_assert!(matches!(
        session
            .journal()
            .operations
            .last()
            .map(|operation| operation.state),
        Some(OperationState::Done)
    ));
    Ok(DiagnosticOutcome {
        temp_written: true,
        link_created: true,
        link_reused: false,
        text_hash: Some(text_hash),
    })
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
    fn link_path_is_beside_the_render_executable() {
        let spec = DiagnosticSpec {
            render_dir: PathBuf::from(r"D:\game\bin"),
            temp_path: PathBuf::from(r"C:\Temp\dlssg_sm86-nte-42.ini"),
            text: String::new(),
        };
        assert_eq!(
            spec.link_path(),
            PathBuf::from(r"D:\game\bin\dlssg_sm86.ini")
        );
    }
}
