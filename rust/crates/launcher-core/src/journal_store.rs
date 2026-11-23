//! 部署记录的落盘与会话（Phase 1 步 7）。
//!
//! 对应现有实现 `DeploymentJournal.ps1`：
//! - `Assert-JournalOrdinaryParents`（:14-26）：父目录链上任一已存在的**重解析点**都拒绝，
//!   避免把记录写进链接目录。
//! - `Save-DeploymentJournal`（:28-49）：先写同目录临时文件，再原子替换；临时文件必删。
//! - 互斥（:72-79、:96-102）：`Global\DLSSG-Deployment-<identity>` 与
//!   `Global\DLSSG-Config-<identity>`，`identity = UPPERHEX(SHA256(UTF8(normalized(render))))`，
//!   `WaitOne(0)` 抢不到立即失败。
//!
//! 与现有实现的差异：原子替换用 `fs::rename`（Windows 上即 `MoveFileEx` +
//! `MOVEFILE_REPLACE_EXISTING`），不复刻 `File.Replace` 的备用流处理（PARITY 未要求）。

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::{Error, Result};
use crate::journal::{Journal, Operation, OperationKind, OperationState, Phase};
use crate::ops::{MutexLease, MutexOps};
use crate::path::PathContext;

/// 记录相对路径：`.state/<gameId>/active.json`。
pub fn state_relative_path(game_id: &str) -> PathBuf {
    Path::new(".state").join(game_id).join("active.json")
}

/// 父目录链上不允许出现重解析点（链接/junction）。
pub fn assert_ordinary_parents(path: &Path) -> Result<()> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(Error::io_plain)?.join(path)
    };
    let mut current = absolute.parent().map(Path::to_path_buf);
    while let Some(directory) = current {
        if let Ok(metadata) = std::fs::symlink_metadata(&directory) {
            if metadata.file_type().is_symlink() {
                return Err(Error::invalid(format!(
                    "Reparse-point parent refused: {}",
                    directory.display()
                )));
            }
        }
        let next = directory.parent().map(Path::to_path_buf);
        match next {
            Some(next) if next != directory => current = Some(next),
            _ => break,
        }
    }
    Ok(())
}

/// 原子写入一条记录。
pub fn save_atomic(path: &Path, journal: &Journal) -> Result<()> {
    assert_ordinary_parents(path)?;
    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        if metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(Error::invalid("Journal path is not an ordinary file."));
        }
    }
    let directory = path
        .parent()
        .ok_or_else(|| Error::invalid(format!("记录路径没有父目录：{}", path.display())))?;
    std::fs::create_dir_all(directory).map_err(|error| Error::io(directory, error))?;

    let text = journal.to_json()?;
    let temporary = directory.join(format!("{}.tmp", unique_token()));
    let bytes = text.into_bytes();
    std::fs::write(&temporary, &bytes).map_err(|error| Error::io(&temporary, error))?;
    let outcome = std::fs::rename(&temporary, path).map_err(|error| Error::io(path, error));
    if outcome.is_err() || temporary.exists() {
        let _ = std::fs::remove_file(&temporary);
    }
    outcome
}

/// 读一条记录；不存在返回 `Ok(None)`。
pub fn load(path: &Path) -> Result<Option<Journal>> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let text = String::from_utf8(bytes)
                .map_err(|_| Error::invalid(format!("记录不是合法 UTF-8：{}", path.display())))?;
            Ok(Some(Journal::from_json(&text)?))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Error::io(path, error)),
    }
}

/// 删除记录（恢复完成后的最后一步）。
pub fn remove(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::io(path, error)),
    }
}

/// 部署互斥名：`Global\DLSSG-Deployment-<identity>`。
pub fn deployment_mutex_name(render: &Path) -> Result<String> {
    Ok(format!("Global\\DLSSG-Deployment-{}", identity(render)?))
}

/// 配置互斥名：`Global\DLSSG-Config-<identity>`（同一 identity）。
pub fn configuration_mutex_name(render: &Path) -> Result<String> {
    Ok(format!("Global\\DLSSG-Config-{}", identity(render)?))
}

/// 部署 identity：渲染目录归一化后的 SHA256 大写十六进制。
pub fn identity(render: &Path) -> Result<String> {
    let text = render
        .to_str()
        .ok_or_else(|| Error::invalid(format!("渲染目录不是合法 UTF-8：{}", render.display())))?;
    let normalized = PathContext::current()?.normalize(text)?;
    Ok(crate::path::sha256_upper(normalized.as_bytes()))
}

/// `SystemTime` → `2026-11-22T13:56:09.5234043Z`（7 位小数 + Z），
/// 等价 `ToUniversalTime().ToString('o')`。
pub fn format_script_started(time: SystemTime) -> String {
    let since_epoch = time.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);
    let seconds = since_epoch.as_secs() as i64;
    let ticks = since_epoch.subsec_nanos() as i64 / 100; // 100ns 单位
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3600;
    let minute = (seconds_of_day % 3600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{ticks:07}Z")
}

/// Howard Hinnant 的 `civil_from_days`：以 1970-01-01 为第 0 天。
pub(crate) fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted / 146_097
    } else {
        (shifted - 146_096) / 146_097
    };
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_position = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_position + 2) / 5 + 1) as u32;
    let month = if month_position < 10 {
        month_position + 3
    } else {
        month_position - 9
    } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

fn unique_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}{:x}", std::process::id())
}

/// 一次部署/恢复的会话：持有记录、两个互斥、以及落盘路径。
pub struct DeploymentSession<'a> {
    ops: &'a dyn MutexOps,
    path: PathBuf,
    journal: Journal,
    deployment_lease: MutexLease<'a>,
    configuration_lease: MutexLease<'a>,
}

impl<'a> DeploymentSession<'a> {
    /// 打开会话：先抢部署互斥，再抢配置互斥（都是 `WaitOne(0)`）。
    pub fn open(
        ops: &'a dyn MutexOps,
        state_root: &Path,
        render: &Path,
        journal: Journal,
    ) -> Result<Self> {
        journal.validate()?;
        let deployment_name = deployment_mutex_name(render)?;
        let configuration_name = configuration_mutex_name(render)?;
        let deployment_lease = MutexLease::acquire(ops, &deployment_name, Duration::from_millis(0))
            .map_err(|error| match error {
                Error::MutexTimeout { .. } => {
                    Error::invalid("Another deployment/recovery is running for this game.")
                }
                other => other,
            })?;
        let configuration_lease =
            MutexLease::acquire(ops, &configuration_name, Duration::from_millis(0))?;
        let path = state_root.join(state_relative_path(&journal.game_id));
        let session = Self {
            ops,
            path,
            journal,
            deployment_lease,
            configuration_lease,
        };
        session.persist()?;
        Ok(session)
    }

    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn mutex_names(&self) -> (&str, &str) {
        (
            self.deployment_lease.name(),
            self.configuration_lease.name(),
        )
    }

    fn persist(&self) -> Result<()> {
        save_atomic(&self.path, &self.journal)
    }

    pub fn set_phase(&mut self, phase: Phase) -> Result<()> {
        self.journal.phase = phase;
        self.persist()
    }

    /// 记录一步操作的 `intent` 并立刻落盘，返回操作下标。
    pub fn start_operation(
        &mut self,
        kind: OperationKind,
        path: impl Into<String>,
    ) -> Result<usize> {
        if self.journal.is_dlssg_only()
            && matches!(
                kind,
                OperationKind::EngineLink | OperationKind::Backup | OperationKind::Runtime
            )
        {
            return Err(Error::invalid("仅 DLSSG 模式禁止画面配置操作。"));
        }
        self.journal
            .operations
            .push(Operation::new(kind, path, OperationState::Intent));
        let index = self.journal.operations.len() - 1;
        self.persist()?;
        Ok(index)
    }

    pub fn set_operation_target(&mut self, index: usize, target: impl Into<String>) -> Result<()> {
        self.operation_mut(index)?.target = target.into();
        self.persist()
    }

    pub fn set_operation_hash(&mut self, index: usize, hash: impl Into<String>) -> Result<()> {
        self.operation_mut(index)?.hash = hash.into();
        self.persist()
    }

    /// 操作完成。
    pub fn complete_operation(&mut self, index: usize) -> Result<()> {
        self.operation_mut(index)?.state = OperationState::Done;
        self.persist()
    }

    /// 该操作已恢复。
    pub fn mark_restored(&mut self, index: usize) -> Result<()> {
        self.operation_mut(index)?.state = OperationState::Restored;
        self.persist()
    }

    fn operation_mut(&mut self, index: usize) -> Result<&mut Operation> {
        let count = self.journal.operations.len();
        if index >= count {
            return Err(Error::invalid(format!(
                "记录里没有第 {index} 条操作（共 {count} 条）"
            )));
        }
        Ok(&mut self.journal.operations[index])
    }

    /// 正常收尾：删除记录并释放互斥。记录删除失败时保留互斥直到 drop。
    pub fn finish(self) -> Result<()> {
        remove(&self.path)?;
        let release = self.configuration_lease.release();
        let release_deployment = self.deployment_lease.release();
        let _ = self.ops;
        release?;
        release_deployment
    }

    /// 只删除记录文件（恢复成功后调用），互斥仍由会话持有到 `finish`。
    pub fn remove_journal(&mut self) -> Result<()> {
        remove(&self.path)
    }
}

impl std::fmt::Debug for DeploymentSession<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DeploymentSession")
            .field("path", &self.path)
            .field("game_id", &self.journal.game_id)
            .field("phase", &self.journal.phase)
            .field("operations", &self.journal.operations.len())
            .finish()
    }
}

/// 旧实现的状态基目录名（其下是 `启动脚本\.state`），只读兜底。
pub const LEGACY_STATE_DIRECTORY: &str = "启动脚本";

/// 状态**基目录**：`DeploymentSession` 会把 `.state/<gameId>/active.json` 拼在它下面。
/// 新程序用启动器根目录本身。
pub fn primary_state_root(root: &Path) -> PathBuf {
    root.to_path_buf()
}

/// 旧实现的状态基目录：`<根>\启动脚本`（记录在 `<根>\启动脚本\.state` 下）。
pub fn legacy_state_root(root: &Path) -> PathBuf {
    root.join(LEGACY_STATE_DIRECTORY)
}

/// 依次尝试的状态基目录：新根优先，旧根兜底。
pub fn state_roots(root: &Path) -> Vec<PathBuf> {
    vec![primary_state_root(root), legacy_state_root(root)]
}

/// 实际使用的状态基目录：谁的 `.state` 存在就用谁，都不存在时返回新根（写入位置）。
pub fn resolve_state_root(root: &Path) -> PathBuf {
    for candidate in state_roots(root) {
        if candidate.join(".state").is_dir() {
            return candidate;
        }
    }
    primary_state_root(root)
}

/// 按双根顺序读取某个游戏的记录，返回命中的路径与内容。
///
/// 新根优先：新程序写入后不会被旧根里的过期记录盖掉；旧根只读兜底，
/// 迁移到新根由调用方决定（M6-01 只负责读得到）。
pub fn load_for_game(root: &Path, game_id: &str) -> Result<Option<(PathBuf, Journal)>> {
    for state_root in state_roots(root) {
        let path = state_root.join(state_relative_path(game_id));
        if let Some(journal) = load(&path)? {
            return Ok(Some((path, journal)));
        }
    }
    Ok(None)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_path_is_game_scoped() {
        let path = state_relative_path("steam-4356430");
        assert_eq!(
            path.to_string_lossy().replace('\\', "/"),
            ".state/steam-4356430/active.json"
        );
    }

    #[test]
    fn mutex_names_follow_the_identity_rule() {
        let render = Path::new(r"D:\SteamLibrary\steamapps\common\NBA 2K27");
        let deployment = deployment_mutex_name(render).expect("部署互斥名");
        let configuration = configuration_mutex_name(render).expect("配置互斥名");
        assert!(deployment.starts_with("Global\\DLSSG-Deployment-"));
        assert!(configuration.starts_with("Global\\DLSSG-Config-"));
        let identity = identity(render).expect("identity");
        assert_eq!(deployment, format!("Global\\DLSSG-Deployment-{identity}"));
        assert_eq!(configuration, format!("Global\\DLSSG-Config-{identity}"));
        assert_eq!(identity.len(), 64);
        assert!(
            identity
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_lowercase())
        );
    }

    #[test]
    fn identity_is_case_and_trailing_separator_insensitive() {
        let a = identity(Path::new(r"D:\Games\Foo")).expect("a");
        let b = identity(Path::new(r"d:\games\foo\")).expect("b");
        assert_eq!(a, b);
    }

    #[test]
    fn script_started_matches_powershell_round_trip_format() {
        // 2026-11-22T13:56:09.5234043Z 对应 UNIX 秒 1795350969 + 523404300 ns
        let time = UNIX_EPOCH + Duration::new(1_795_355_769, 523_404_300);
        assert_eq!(format_script_started(time), "2026-11-22T13:56:09.5234043Z");
        assert_eq!(
            format_script_started(UNIX_EPOCH),
            "1970-01-01T00:00:00.0000000Z"
        );
    }

    #[test]
    fn civil_conversion_handles_leap_day() {
        let time = UNIX_EPOCH + Duration::from_secs(951_782_400); // 2000-02-29T00:00:00Z
        assert_eq!(format_script_started(time), "2000-02-29T00:00:00.0000000Z");
    }
}

#[cfg(test)]
mod dual_root_tests {
    use super::*;
    use std::path::PathBuf;

    struct Tree(PathBuf);

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tree(name: &str) -> Tree {
        let root = std::env::temp_dir().join(format!("dlssg-dual-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("根目录");
        Tree(root)
    }

    fn write_record(state_root: &Path, game_id: &str, session: &str) {
        let mut journal =
            Journal::from_json(&crate::fixtures::read_text("journal/01-new.json")).expect("样本");
        journal.game_id = game_id.to_owned();
        journal.session = session.to_owned();
        let relative = state_relative_path(game_id);
        let path = state_root.join(relative);
        std::fs::create_dir_all(path.parent().expect("父目录")).expect("目录");
        std::fs::write(
            path,
            serde_json::to_string_pretty(&journal).expect("序列化"),
        )
        .expect("写");
    }

    #[test]
    fn primary_root_wins_over_legacy() {
        let tree = tree("primary");
        write_record(
            &primary_state_root(&tree.0),
            "steam-1",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        );
        write_record(
            &legacy_state_root(&tree.0),
            "steam-1",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        );

        assert_eq!(resolve_state_root(&tree.0), primary_state_root(&tree.0));
        let (path, journal) = load_for_game(&tree.0, "steam-1")
            .expect("读取")
            .expect("应命中");
        assert_eq!(journal.session, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert!(path.starts_with(primary_state_root(&tree.0).join(".state")));
    }

    #[test]
    fn legacy_root_is_used_when_primary_missing() {
        let tree = tree("legacy");
        write_record(
            &legacy_state_root(&tree.0),
            "steam-2",
            "cccccccccccccccccccccccccccccccc",
        );

        assert_eq!(resolve_state_root(&tree.0), legacy_state_root(&tree.0));
        let (path, journal) = load_for_game(&tree.0, "steam-2")
            .expect("读取")
            .expect("应命中");
        assert_eq!(journal.game_id, "steam-2");
        assert!(path.starts_with(legacy_state_root(&tree.0).join(".state")));
    }

    #[test]
    fn unknown_game_and_empty_roots_return_none() {
        let tree = tree("none");
        assert!(load_for_game(&tree.0, "steam-404").expect("读取").is_none());
        std::fs::create_dir_all(primary_state_root(&tree.0).join(".state")).expect("建目录");
        assert!(load_for_game(&tree.0, "steam-404").expect("读取").is_none());
        assert_eq!(resolve_state_root(&tree.0), primary_state_root(&tree.0));
    }

    #[test]
    fn real_workspace_record_is_readable_when_present() {
        // 只读校验：仓库里可能留有真实部署记录（例如 steam-4356430），能解析就说明双根逻辑对得上。
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .and_then(std::path::Path::parent)
            .map(|value| value.to_path_buf());
        let Some(root) = root else { return };
        let legacy = legacy_state_root(&root).join(".state");
        if !legacy.is_dir() {
            eprintln!("跳过：本工作区没有旧状态根（{}）", legacy.display());
            return;
        }
        let mut found = 0;
        for entry in std::fs::read_dir(&legacy).expect("读目录") {
            let entry = entry.expect("条目");
            if !entry.path().is_dir() {
                continue;
            }
            let Some(game_id) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if let Some((path, journal)) = load_for_game(&root, &game_id).expect("读取") {
                assert_eq!(journal.game_id, game_id);
                assert_eq!(journal.version, 1);
                eprintln!(
                    "命中真实记录：{}（{} 个操作）",
                    path.display(),
                    journal.operations.len()
                );
                found += 1;
            }
        }
        eprintln!("旧状态根下可读记录数：{found}");
    }
}
