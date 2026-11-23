//! 资源与代理部署（Phase 1 步 8–9）。
//!
//! 对应现有实现 `LauncherResources.ps1:64-165` 与 `Run-*.ps1` 里的 `Assert-LinkOrAbsent`：
//! - 6 个代理名严格映射到 `1..6`，只有 `version.dll` 在资源根，其余在 `alternatives\`
//! - 5 个资源目录链接：`<渲染目录>\dlssg_sm86\{runtime,backend,kernels,xefg,sr}`
//! - 断言分层（PARITY N-5）：核心 5 项永远要求；`sr` 两项只在超分未关闭时要求
//! - 其他代理名位置出现**指向本项目的旧链接**要停下（先恢复）；普通文件属第三方，记录后共存
//! - `_storage_\<proxy>` 只有哈希与项目代理一致时才认领
//!
//! 所有链接操作都走 [`SymlinkOps`]，因此测试不需要管理员权限。

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::journal::OperationKind;
use crate::journal_store::DeploymentSession;
use crate::ops::SymlinkOps;

/// 代理编号到文件名的严格映射（`1` 起）。
pub const PROXY_NAMES: [&str; 6] = [
    "version.dll",
    "winmm.dll",
    "dbghelp.dll",
    "dinput8.dll",
    "dxgi.dll",
    "d3d12.dll",
];

/// 资源目录顺序与 `LauncherResources.ps1:77` 一致。
pub const RESOURCE_DIRS: [&str; 5] = ["runtime", "backend", "kernels", "xefg", "sr"];

/// 永远要求的资源（相对启动器根）。
pub const CORE_REQUIRED: [&str; 5] = [
    "dlssg_sm86.ini",
    "dlssg_sm86\\runtime\\nvngx_dlssg.dll",
    "dlssg_sm86\\backend\\sm86_backend.dll",
    "dlssg_sm86\\kernels\\fg_kernels.dll",
    "dlssg_sm86\\xefg\\xefg_kernels.dll",
];

/// 只有超分未关闭时才要求的资源。
pub const SUPER_RESOLUTION_REQUIRED: [&str; 2] = [
    "dlssg_sm86\\sr\\sr_kernels.dll",
    "dlssg_sm86\\sr\\nvngx_dlss.dll",
];

/// 代理选择。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProxyChoice {
    index: usize,
}

impl ProxyChoice {
    /// `代理DLL=1..6`，省略时是 1（`version.dll`）。
    pub fn from_index(index: &str) -> Result<Self> {
        let trimmed = index.trim();
        let parsed: usize = trimmed
            .parse()
            .map_err(|_| Error::invalid(format!("代理DLL 编号非法：{trimmed}")))?;
        if !(1..=PROXY_NAMES.len()).contains(&parsed) {
            return Err(Error::invalid(format!(
                "代理DLL 编号超出范围（1..{}）：{trimmed}",
                PROXY_NAMES.len()
            )));
        }
        Ok(Self { index: parsed - 1 })
    }

    pub fn from_name(name: &str) -> Result<Self> {
        let wanted = name.trim().to_ascii_lowercase();
        PROXY_NAMES
            .iter()
            .position(|candidate| candidate.eq_ignore_ascii_case(&wanted))
            .map(|index| Self { index })
            .ok_or_else(|| Error::invalid(format!("不是受支持的代理名：{name}")))
    }

    pub fn default_choice() -> Self {
        Self { index: 0 }
    }

    /// 1 起始编号，写进日志用。
    pub fn number(self) -> usize {
        self.index + 1
    }

    pub fn name(self) -> &'static str {
        PROXY_NAMES[self.index]
    }

    /// 相对启动器根的路径：`version.dll` 或 `alternatives\<name>`。
    pub fn relative(self) -> PathBuf {
        if self.index == 0 {
            PathBuf::from(self.name())
        } else {
            Path::new("alternatives").join(self.name())
        }
    }

    pub fn project_path(self, project_root: &Path) -> PathBuf {
        project_root.join(self.relative())
    }
}

/// 一条链接规格。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSpec {
    pub path: PathBuf,
    pub target: PathBuf,
    pub target_is_dir: bool,
}

/// 一个渲染目录的部署视图。
#[derive(Debug, Clone)]
pub struct Deployment<'a> {
    pub project_root: &'a Path,
    pub render_dir: &'a Path,
    pub proxy: ProxyChoice,
}

impl<'a> Deployment<'a> {
    pub fn new(project_root: &'a Path, render_dir: &'a Path, proxy: ProxyChoice) -> Self {
        Self {
            project_root,
            render_dir,
            proxy,
        }
    }

    /// `<渲染目录>\dlssg_sm86`
    pub fn resource_root(&self) -> PathBuf {
        self.render_dir.join("dlssg_sm86")
    }

    /// 代理链接位置：`<渲染目录>\<代理名>`
    pub fn proxy_path(&self) -> PathBuf {
        self.render_dir.join(self.proxy.name())
    }

    /// 代理在项目里的真实文件。
    pub fn proxy_target(&self) -> PathBuf {
        self.proxy.project_path(self.project_root)
    }

    /// `_storage_\<proxy>` 副本位置。
    pub fn storage_dll(&self) -> PathBuf {
        self.render_dir.join("_storage_").join(self.proxy.name())
    }

    /// 5 个资源目录链接。
    pub fn resource_specs(&self) -> Vec<LinkSpec> {
        RESOURCE_DIRS
            .iter()
            .map(|name| LinkSpec {
                path: self.resource_root().join(name),
                target: self.project_root.join("dlssg_sm86").join(name),
                target_is_dir: true,
            })
            .collect()
    }

    /// 代理链接。
    pub fn proxy_spec(&self) -> LinkSpec {
        LinkSpec {
            path: self.proxy_path(),
            target: self.proxy_target(),
            target_is_dir: false,
        }
    }

    /// 部署与恢复都要认的链接集合：代理在前，与 `linkSpecs[0]` 一致。
    pub fn all_specs(&self) -> Vec<LinkSpec> {
        let mut specs = vec![self.proxy_spec()];
        specs.extend(self.resource_specs());
        specs
    }
}

/// 按超分是否启用给出必需文件清单。
pub fn required_files(super_resolution: bool) -> Vec<&'static str> {
    let mut required = CORE_REQUIRED.to_vec();
    if super_resolution {
        required.extend(SUPER_RESOLUTION_REQUIRED);
    }
    required
}

/// 必需资源是否齐全。缺失时指出第一个缺的文件。
pub fn assert_required_files(project_root: &Path, super_resolution: bool) -> Result<()> {
    for relative in required_files(super_resolution) {
        let path = project_root.join(relative);
        if !path.is_file() {
            return Err(Error::invalid(format!(
                "Missing DLSSG resource: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

/// 其他代理名位置的扫描结果。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProxyScan {
    /// 指向本项目、但当前未选中的旧链接：必须先恢复，不能直接部署。
    pub stale: Vec<PathBuf>,
    /// 非本项目的普通文件（mod 注入器等）：只记录，共存加载。
    pub foreign: Vec<PathBuf>,
}

/// 扫描除所选代理之外的其他 5 个代理名位置。
pub fn scan_other_proxies(deployment: &Deployment<'_>, ops: &dyn SymlinkOps) -> Result<ProxyScan> {
    let mut scan = ProxyScan::default();
    let project_prefix = project_prefix(deployment.project_root)?;
    for name in PROXY_NAMES {
        if name == deployment.proxy.name() {
            continue;
        }
        let path = deployment.render_dir.join(name);
        if !exists_as_link_or_file(ops, &path)? {
            continue;
        }
        let info = ops.inspect(&path)?;
        let points_into_project = match &info {
            Some(link) => {
                let target = crate::path::PathContext::current()?
                    .full_path(&link.target.to_string_lossy())?;
                let target = target.trim_end_matches('\\').to_lowercase();
                info_kind_matches(link, &target, &project_prefix)
            }
            None => false,
        };
        if points_into_project {
            scan.stale.push(path);
        } else {
            scan.foreign.push(path);
        }
    }
    Ok(scan)
}

fn info_kind_matches(link: &crate::ops::LinkInfo, target: &str, project_prefix: &str) -> bool {
    link.kind == crate::ops::LinkKind::SymbolicLink && target.starts_with(project_prefix)
}

/// 部署前检查：资源齐全、旧链接不残留、资源根是普通目录、5 个资源位置无冲突。
///
/// 对应 `Assert-DlssgDeployment`（`LauncherResources.ps1:84-122`）。
pub fn assert_deployment(
    deployment: &Deployment<'_>,
    super_resolution: bool,
    ops: &dyn SymlinkOps,
) -> Result<ProxyScan> {
    assert_required_files(deployment.project_root, super_resolution)?;

    let scan = scan_other_proxies(deployment, ops)?;
    if let Some(path) = scan.stale.first() {
        return Err(Error::invalid(format!(
            "Stale deployment link must be restored first: {}",
            path.display()
        )));
    }

    let root = deployment.resource_root();
    if let Ok(metadata) = std::fs::symlink_metadata(&root) {
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(Error::invalid(format!(
                "Resource root must be an ordinary directory: {}",
                root.display()
            )));
        }
    }

    for spec in deployment.resource_specs() {
        if !exists_as_link_or_file(ops, &spec.path)? {
            continue;
        }
        let matches = match ops.inspect(&spec.path)? {
            Some(link) => {
                link.kind == crate::ops::LinkKind::SymbolicLink
                    && same_path(&link.target, &spec.target)?
            }
            None => false,
        };
        if !matches {
            return Err(Error::invalid(format!(
                "Resource path conflict (preserved): {}",
                spec.path.display()
            )));
        }
    }
    Ok(scan)
}

/// 一次资源安装的结果。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct InstallOutcome {
    pub created: Vec<LinkSpec>,
    pub reused: Vec<LinkSpec>,
    pub directory_created: Option<PathBuf>,
}

/// 部署 5 个资源目录链接，把每一步写进记录。
///
/// 对应 `Install-DlssgResources`（`LauncherResources.ps1:124-144`）：
/// 已存在的正确链接按"复用"登记，不重复创建；资源根不存在时先建目录并记为 `directory`。
pub fn install_resources(
    session: &mut DeploymentSession<'_>,
    ops: &dyn SymlinkOps,
    deployment: &Deployment<'_>,
    dry_run: bool,
) -> Result<InstallOutcome> {
    let mut outcome = InstallOutcome::default();
    for spec in deployment.resource_specs() {
        if exists_as_link_or_file(ops, &spec.path)? {
            if !dry_run {
                register_reused_link(session, &spec)?;
            }
            outcome.reused.push(spec);
            continue;
        }
        if dry_run {
            outcome.created.push(spec);
            continue;
        }
        if !path_exists(&deployment.resource_root())
            && ops.inspect(&deployment.resource_root())?.is_none()
        {
            let index = session.start_operation(
                OperationKind::Directory,
                path_text(&deployment.resource_root())?,
            )?;
            let root_path = deployment.resource_root();
            std::fs::create_dir(&root_path).map_err(|error| Error::io(&root_path, error))?;
            session.complete_operation(index)?;
            outcome.directory_created = Some(deployment.resource_root());
        }
        let index = session.start_operation(OperationKind::Link, path_text(&spec.path)?)?;
        session.set_operation_target(index, path_text(&spec.target)?)?;
        ops.create(&spec.path, &spec.target, spec.target_is_dir)?;
        session.complete_operation(index)?;
        outcome.created.push(spec);
    }
    Ok(outcome)
}

/// 部署代理链接（`<渲染目录>\<代理名>` → 项目里的代理文件）。
///
/// 对应 `Select-DlssgProxy` 之后的 `Ensure-Link`：已存在且指向同一目标就复用，
/// 否则记一条 `link` 操作再建链接。
pub fn install_proxy_link(
    session: &mut DeploymentSession<'_>,
    ops: &dyn SymlinkOps,
    deployment: &Deployment<'_>,
    dry_run: bool,
) -> Result<LinkOutcome> {
    let spec = deployment.proxy_spec();
    if let Some(info) = ops.inspect(&spec.path)? {
        let matches = info.kind == crate::ops::LinkKind::SymbolicLink
            && same_path(&info.target, &spec.target)?;
        if !matches {
            return Err(Error::invalid(format!(
                "代理位置已被占用且不是本次的链接：{}",
                spec.path.display()
            )));
        }
        if !dry_run {
            register_reused_link(session, &spec)?;
        }
        return Ok(LinkOutcome::Reused);
    }
    if std::fs::symlink_metadata(&spec.path).is_ok() {
        return Err(Error::invalid(format!(
            "代理位置已有普通文件（第三方代理），不做覆盖：{}",
            spec.path.display()
        )));
    }
    if dry_run {
        return Ok(LinkOutcome::Created);
    }
    let index = session.start_operation(OperationKind::Link, path_text(&spec.path)?)?;
    session.set_operation_target(index, path_text(&spec.target)?)?;
    ops.create(&spec.path, &spec.target, false)?;
    session.complete_operation(index)?;
    Ok(LinkOutcome::Created)
}

/// 代理链接的部署结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkOutcome {
    Created,
    Reused,
}

/// 拆除资源链接：只删链接本身，资源根只在空时删除。
///
/// 对应 `Undo-DlssgResources`（`LauncherResources.ps1:146-165`）。
pub fn undo_resources(ops: &dyn SymlinkOps, deployment: &Deployment<'_>) -> Result<UndoOutcome> {
    let mut outcome = UndoOutcome::default();
    for spec in deployment.resource_specs() {
        if !exists_as_link_or_file(ops, &spec.path)? {
            continue;
        }
        let is_ours = match ops.inspect(&spec.path)? {
            Some(link) => {
                link.kind == crate::ops::LinkKind::SymbolicLink
                    && same_path(&link.target, &spec.target)?
            }
            None => false,
        };
        if !is_ours {
            outcome.preserved.push(spec.path);
            continue;
        }
        ops.remove(&spec.path)?;
        outcome.removed.push(spec.path);
    }
    let root = deployment.resource_root();
    if root.is_dir() && is_directory_empty(&root)? {
        std::fs::remove_dir(&root).map_err(|error| Error::io(&root, error))?;
        outcome.root_removed = true;
    }
    Ok(outcome)
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UndoOutcome {
    pub removed: Vec<PathBuf>,
    pub preserved: Vec<PathBuf>,
    pub root_removed: bool,
}

fn register_reused_link(session: &mut DeploymentSession<'_>, spec: &LinkSpec) -> Result<()> {
    let path = path_text(&spec.path)?;
    let target = path_text(&spec.target)?;
    let already = session.journal().operations.iter().any(|operation| {
        operation.kind == OperationKind::Link && same_text(&operation.path, &path)
    });
    if already {
        return Ok(());
    }
    let index = session.start_operation(OperationKind::Link, path)?;
    session.set_operation_target(index, target)?;
    session.complete_operation(index)
}

fn path_text(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::invalid(format!("路径不是合法 UTF-8：{}", path.display())))
}

fn same_text(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

fn same_path(left: &Path, right: &Path) -> Result<bool> {
    let context = crate::path::PathContext::current()?;
    Ok(context.normalize(&left.to_string_lossy())?
        == context.normalize(&right.to_string_lossy())?)
}

fn project_prefix(project_root: &Path) -> Result<String> {
    let context = crate::path::PathContext::current()?;
    let normalized = context.normalize(&project_root.to_string_lossy())?;
    Ok(format!("{normalized}\\"))
}

/// 恢复收尾检查：本启动器部署的东西不能留在游戏目录里（日志除外）。
///
/// 对应 `Assert-DeploymentResourcesRemoved`（`DeploymentJournal.ps1:369-379`）：
/// - 6 条链接（代理 + 5 个资源目录）都不能存在
/// - 非 DLSSG 模式下 `Engine.ini` 的运行时副本与备份也不能存在
/// - `_storage_\<proxy>` 若仍在，哈希不能再等于项目代理（等于就说明是我们的残留）
///
/// 现有实现把这一步放在删除记录**之前**；core 的 `restore` 没有部署上下文，
/// 因此由调用方在删除记录前调用本函数（见 PARITY-REPORT 的 D-9）。
pub fn assert_resources_removed(
    deployment: &Deployment<'_>,
    engine_runtime: Option<&Path>,
    engine_backup: Option<&Path>,
    ops: &dyn SymlinkOps,
) -> Result<()> {
    for spec in deployment.all_specs() {
        if exists_as_link_or_file(ops, &spec.path)? {
            return Err(Error::invalid(format!(
                "Deployment residue preserved; recovery incomplete: {}",
                spec.path.display()
            )));
        }
    }
    for path in [engine_runtime, engine_backup].into_iter().flatten() {
        if path_exists(path) {
            return Err(Error::invalid(format!(
                "Deployment residue preserved; recovery incomplete: {}",
                path.display()
            )));
        }
    }
    if let StorageClaim::Owned(_) = claim_storage(deployment)? {
        return Err(Error::invalid(format!(
            "Owned storage DLL remains: {}",
            deployment.storage_dll().display()
        )));
    }
    Ok(())
}

/// 判断路径是否存在（含悬空链接）。
fn path_exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// 链接能力里登记过、或磁盘上真实存在，都算"已存在"。
/// 这样测试可以只用假链接实现，不必真的建链接。
fn exists_as_link_or_file(ops: &dyn SymlinkOps, path: &Path) -> Result<bool> {
    if path_exists(path) {
        return Ok(true);
    }
    Ok(ops.inspect(path)?.is_some())
}

/// `_storage_\<proxy>` 的归属。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageClaim {
    /// 不存在，无需处理。
    Absent,
    /// 与项目代理哈希一致：属于本启动器的旧副本。
    Owned(String),
    /// 哈希不一致：保留并告警。
    Foreign(String),
}

/// 按哈希认领 `_storage_\<proxy>`（PARITY C6）。
pub fn claim_storage(deployment: &Deployment<'_>) -> Result<StorageClaim> {
    let storage = deployment.storage_dll();
    let Some(hash) = crate::path::file_hash_or_none(&storage)? else {
        return Ok(StorageClaim::Absent);
    };
    let project_hash = crate::path::file_hash_or_none(&deployment.proxy_target())?;
    match project_hash {
        Some(project) if project == hash => Ok(StorageClaim::Owned(hash)),
        _ => Ok(StorageClaim::Foreign(hash)),
    }
}

fn is_directory_empty(path: &Path) -> Result<bool> {
    let mut entries = std::fs::read_dir(path).map_err(|error| Error::io(path, error))?;
    Ok(entries.next().is_none())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_indices_follow_the_documented_order() {
        let expected = [
            (1, "version.dll"),
            (2, "winmm.dll"),
            (3, "dbghelp.dll"),
            (4, "dinput8.dll"),
            (5, "dxgi.dll"),
            (6, "d3d12.dll"),
        ];
        for (index, name) in expected {
            let choice = ProxyChoice::from_index(&index.to_string()).expect("编号");
            assert_eq!(choice.name(), name);
            assert_eq!(choice.number(), index);
        }
        assert_eq!(ProxyChoice::default_choice().name(), "version.dll");
        assert_eq!(
            ProxyChoice::from_index("1").expect("1").relative(),
            PathBuf::from("version.dll")
        );
        assert_eq!(
            ProxyChoice::from_index("2").expect("2").relative(),
            PathBuf::from("alternatives\\winmm.dll")
        );
        assert!(ProxyChoice::from_index("0").is_err());
        assert!(ProxyChoice::from_index("7").is_err());
        assert!(ProxyChoice::from_index("version.dll").is_err());
        assert!(ProxyChoice::from_name("nope.dll").is_err());
        assert_eq!(
            ProxyChoice::from_name("DXGI.DLL")
                .expect("大小写不敏感")
                .name(),
            "dxgi.dll"
        );
    }

    #[test]
    fn required_files_depend_on_super_resolution() {
        assert_eq!(required_files(false).len(), 5);
        assert_eq!(required_files(true).len(), 7);
        assert!(
            !required_files(false)
                .iter()
                .any(|item| item.contains("sr\\"))
        );
    }

    #[test]
    fn specs_put_the_proxy_first_and_cover_five_directories() {
        let deployment = Deployment::new(
            Path::new(r"D:\launcher"),
            Path::new(r"D:\game"),
            ProxyChoice::from_index("2").expect("2"),
        );
        let specs = deployment.all_specs();
        assert_eq!(specs.len(), 6);
        assert_eq!(specs[0].path, PathBuf::from(r"D:\game\winmm.dll"));
        assert_eq!(
            specs[0].target,
            PathBuf::from(r"D:\launcher\alternatives\winmm.dll")
        );
        assert!(!specs[0].target_is_dir);
        let directories: Vec<String> = specs[1..]
            .iter()
            .map(|spec| {
                spec.path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(
            directories,
            vec!["runtime", "backend", "kernels", "xefg", "sr"]
        );
        assert!(specs[1..].iter().all(|spec| spec.target_is_dir));
        assert_eq!(
            deployment.storage_dll(),
            PathBuf::from(r"D:\game\_storage_\winmm.dll")
        );
    }
}
