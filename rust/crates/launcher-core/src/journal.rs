//! 部署记录（`.state/<gameId>/active.json`，Phase 1 步 6）。
//!
//! 对应现有实现 `DeploymentJournal.ps1:184-259`。契约见 PARITY C1：
//! 字段名、类型、`null` 与空串的区分、`Operations[]` 的顺序与 `State` 流转必须一致。
//!
//! **只保证字段级等价**（PARITY D-1）：PowerShell 的 `ConvertTo-Json` 会写出
//! `"Key":  value`（冒号后两个空格）、空数组跨四行等形态，这里不复刻。
//! 读入时对未知字段宽容（旧版本可能多写字段），对已知字段严格。

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// 当前写出的 schema 版本。读到其他版本一律拒绝，避免误改现场。
pub const SCHEMA_VERSION: u32 = 1;

/// 部署阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    #[serde(rename = "deploying")]
    Deploying,
    #[serde(rename = "recovering")]
    Recovering,
    /// `-KeepLinks` 之后保留部署；三款游戏脚本都会写这个阶段（`Set-DeploymentPhase ''retained''`）。
    #[serde(rename = "retained")]
    Retained,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Deploying => "deploying",
            Phase::Recovering => "recovering",
            Phase::Retained => "retained",
        }
    }
}

/// 记录类型。顺序即恢复顺序（PARITY C3），但记录本身按发生顺序保存。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationKind {
    #[serde(rename = "link")]
    Link,
    #[serde(rename = "engineLink")]
    EngineLink,
    #[serde(rename = "backup")]
    Backup,
    #[serde(rename = "runtime")]
    Runtime,
    #[serde(rename = "storage")]
    Storage,
    #[serde(rename = "diagnostic")]
    Diagnostic,
    #[serde(rename = "directory")]
    Directory,
}

impl OperationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            OperationKind::Link => "link",
            OperationKind::EngineLink => "engineLink",
            OperationKind::Backup => "backup",
            OperationKind::Runtime => "runtime",
            OperationKind::Storage => "storage",
            OperationKind::Diagnostic => "diagnostic",
            OperationKind::Directory => "directory",
        }
    }

    /// 恢复顺序：`engineLink → backup → runtime → storage → link → diagnostic → directory`。
    /// 同类内部逆序（后建先拆），由恢复流程处理。
    pub const RESTORE_ORDER: [OperationKind; 7] = [
        OperationKind::EngineLink,
        OperationKind::Backup,
        OperationKind::Runtime,
        OperationKind::Storage,
        OperationKind::Link,
        OperationKind::Diagnostic,
        OperationKind::Directory,
    ];
}

/// 单步状态。`intent` 先落盘，成功后 `done`，恢复成功后 `restored`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationState {
    #[serde(rename = "intent")]
    Intent,
    #[serde(rename = "done")]
    Done,
    #[serde(rename = "restored")]
    Restored,
}

impl OperationState {
    pub fn as_str(self) -> &'static str {
        match self {
            OperationState::Intent => "intent",
            OperationState::Done => "done",
            OperationState::Restored => "restored",
        }
    }
}

/// 一条操作记录。`Target` 与 `Hash` 在无意义时是**空串**而不是 `null`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Operation {
    #[serde(rename = "Kind")]
    pub kind: OperationKind,
    #[serde(rename = "Path")]
    pub path: String,
    #[serde(rename = "Target", default)]
    pub target: String,
    #[serde(rename = "Hash", default)]
    pub hash: String,
    #[serde(rename = "State")]
    pub state: OperationState,
}

impl Operation {
    pub fn new(kind: OperationKind, path: impl Into<String>, state: OperationState) -> Self {
        Self {
            kind,
            path: path.into(),
            target: String::new(),
            hash: String::new(),
            state,
        }
    }

    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.target = target.into();
        self
    }

    pub fn with_hash(mut self, hash: impl Into<String>) -> Self {
        self.hash = hash.into();
        self
    }
}

/// 旧记录里"复用链接"的条目，仅 `Path` 与 `Target`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReusedLink {
    #[serde(rename = "Path")]
    pub path: String,
    #[serde(rename = "Target")]
    pub target: String,
}

/// 部署模式。仅有 DLSSG（不动画面配置）一种，出现即表示"不托管 `Engine.ini`"。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeploymentMode {
    #[serde(rename = "DlssgOnly")]
    DlssgOnly,
}

/// 一份部署记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    #[serde(rename = "Version")]
    pub version: u32,
    #[serde(rename = "GameId")]
    pub game_id: String,
    #[serde(rename = "Session")]
    pub session: String,
    #[serde(rename = "ScriptPid")]
    pub script_pid: u32,
    /// `SystemTime` 的 ISO-8601 UTC 形式，7 位小数 + `Z`。
    #[serde(rename = "ScriptStarted")]
    pub script_started: String,
    #[serde(rename = "Render")]
    pub render: String,
    #[serde(rename = "Engine")]
    pub engine: String,
    #[serde(rename = "Runtime")]
    pub runtime: String,
    #[serde(rename = "Backup")]
    pub backup: String,
    #[serde(rename = "OriginalExisted")]
    pub original_existed: bool,
    /// 原件不存在时为 `null`（不是空串）。
    #[serde(rename = "OriginalHash", default)]
    pub original_hash: Option<String>,
    #[serde(rename = "StorageExisted")]
    pub storage_existed: bool,
    #[serde(rename = "Phase")]
    pub phase: Phase,
    #[serde(rename = "Operations", default)]
    pub operations: Vec<Operation>,
    #[serde(rename = "Reused", default)]
    pub reused: Vec<ReusedLink>,
    #[serde(
        rename = "DeploymentMode",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub deployment_mode: Option<DeploymentMode>,
    #[serde(rename = "ProxyName", default, skip_serializing_if = "Option::is_none")]
    pub proxy_name: Option<String>,
}

impl Journal {
    /// 从 JSON 文本读入。只做结构解析与版本检查；
    /// 会话标识等语义校验由 [`Journal::validate`] 在真正使用前完成
    /// （样本里 `Session` 是占位符，不能因此解析失败）。
    pub fn from_json(text: &str) -> Result<Self> {
        let journal: Journal = serde_json::from_str(text)
            .map_err(|error| Error::invalid(format!("部署记录不是合法 JSON：{error}")))?;
        journal.check_version()?;
        Ok(journal)
    }

    /// 序列化为便于人读的 JSON。
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|error| Error::invalid(format!("部署记录无法序列化：{error}")))
    }

    /// 完整校验：版本、GameId、会话标识。恢复流程必须在使用记录前调用。
    pub fn validate(&self) -> Result<()> {
        self.check_version()?;
        if self.game_id.trim().is_empty() {
            return Err(Error::invalid("部署记录缺少 GameId。"));
        }
        if !is_valid_session(&self.session) {
            return Err(Error::invalid(format!(
                "部署记录会话标识非法：{}",
                self.session
            )));
        }
        Ok(())
    }

    /// 读到不认识的版本要立刻停手，不能猜。
    fn check_version(&self) -> Result<()> {
        if self.version != SCHEMA_VERSION {
            return Err(Error::unsupported("部署记录版本", self.version.to_string()));
        }
        Ok(())
    }

    /// 某类操作，保持记录顺序。
    pub fn operations_of(&self, kind: OperationKind) -> Vec<&Operation> {
        self.operations
            .iter()
            .filter(|operation| operation.kind == kind)
            .collect()
    }

    /// 是否还有未恢复的操作（`intent` 或 `done`）。
    pub fn has_pending_operations(&self) -> bool {
        self.operations
            .iter()
            .any(|operation| operation.state != OperationState::Restored)
    }

    /// 恢复顺序（下标版）：按类别分组，组内**逆序**（后建先拆）。
    /// 恢复流程要边执行边改 `State`，因此返回下标而不是引用。
    pub fn restore_order_indices(&self) -> Vec<usize> {
        let mut ordered = Vec::with_capacity(self.operations.len());
        for kind in OperationKind::RESTORE_ORDER {
            let mut group: Vec<usize> = self
                .operations
                .iter()
                .enumerate()
                .filter(|(_, operation)| operation.kind == kind)
                .map(|(index, _)| index)
                .collect();
            group.reverse();
            ordered.extend(group);
        }
        ordered
    }

    /// 恢复顺序：按类别分组，组内**逆序**（后建先拆）。
    pub fn restore_sequence(&self) -> Vec<&Operation> {
        let mut ordered = Vec::with_capacity(self.operations.len());
        for kind in OperationKind::RESTORE_ORDER {
            let mut group: Vec<&Operation> = self.operations_of(kind);
            group.reverse();
            ordered.extend(group);
        }
        ordered
    }

    /// DLSSG 模式（不托管 `Engine.ini`）。
    pub fn is_dlssg_only(&self) -> bool {
        self.deployment_mode == Some(DeploymentMode::DlssgOnly)
    }
}

/// 会话标识：32 位十六进制（`[guid]::NewGuid().ToString('N')`）。
pub fn is_valid_session(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

impl Journal {
    /// 新部署的起始记录：`phase=deploying`、操作列表为空。
    ///
    /// `session` 必须是 32 位小写十六进制（`validate` 会再查一次），`script_started`
    /// 用 [`crate::journal_store::format_script_started`] 生成。
    #[allow(clippy::too_many_arguments)]
    pub fn initialize(
        game_id: &str,
        session: &str,
        script_pid: u32,
        script_started: String,
        render: &Path,
        engine: &Path,
        runtime: &Path,
        backup: &Path,
        original_existed: bool,
        original_hash: Option<String>,
        storage_existed: bool,
        deployment_mode: Option<DeploymentMode>,
        proxy_name: &str,
    ) -> Self {
        Self {
            version: 1,
            game_id: game_id.to_owned(),
            session: session.to_owned(),
            script_pid,
            script_started,
            render: render.to_string_lossy().into_owned(),
            engine: engine.to_string_lossy().into_owned(),
            runtime: runtime.to_string_lossy().into_owned(),
            backup: backup.to_string_lossy().into_owned(),
            original_existed,
            original_hash,
            storage_existed,
            phase: Phase::Deploying,
            operations: Vec::new(),
            reused: Vec::new(),
            deployment_mode,
            proxy_name: Some(proxy_name.to_owned()),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn load(relative: &str) -> Journal {
        let text = crate::fixtures::read_text(relative);
        Journal::from_json(&text).unwrap_or_else(|error| panic!("{relative}：{error}"))
    }

    #[test]
    fn new_journal_has_no_operations_and_keeps_the_original_hash() {
        let journal = load("journal/01-new.json");
        assert_eq!(journal.version, SCHEMA_VERSION);
        assert_eq!(journal.game_id, "fixture");
        assert_eq!(journal.phase, Phase::Deploying);
        assert!(journal.operations.is_empty());
        assert!(journal.reused.is_empty());
        assert!(journal.original_existed);
        assert_eq!(
            journal.original_hash.as_deref(),
            Some("BB2D74551F712EA4D0CD4E284A9A5C4CAF6F0E55608B3D6CA388814981EDA50C")
        );
        assert!(!journal.is_dlssg_only());
        assert!(!journal.has_pending_operations());
    }

    #[test]
    fn runtime_stage_records_one_done_operation() {
        let journal = load("journal/02-runtime-done.json");
        assert_eq!(journal.operations.len(), 1);
        assert_eq!(journal.operations[0].kind, OperationKind::Runtime);
        assert_eq!(journal.operations[0].state, OperationState::Done);
        assert_eq!(journal.operations[0].target, "");
        assert_eq!(journal.operations[0].hash, "");
    }

    #[test]
    fn engine_link_intent_is_kept_as_intent() {
        let journal = load("journal/03-engine-link-intent.json");
        let kinds: Vec<OperationKind> = journal
            .operations
            .iter()
            .map(|operation| operation.kind)
            .collect();
        assert_eq!(
            kinds,
            vec![
                OperationKind::Runtime,
                OperationKind::Backup,
                OperationKind::EngineLink
            ]
        );
        let states: Vec<OperationState> = journal
            .operations
            .iter()
            .map(|operation| operation.state)
            .collect();
        assert_eq!(
            states,
            vec![
                OperationState::Done,
                OperationState::Done,
                OperationState::Intent
            ]
        );
        let engine_link = &journal.operations[2];
        assert!(engine_link.target.ends_with("Engine.runtime.ini"));
        assert_eq!(journal.operations[1].hash.len(), 64);
        assert!(journal.has_pending_operations());
    }

    #[test]
    fn partial_failure_keeps_the_pending_storage_operation() {
        let journal = load("journal/04-partial-failure.json");
        let storage = journal.operations_of(OperationKind::Storage);
        assert_eq!(storage.len(), 1);
        assert_eq!(storage[0].state, OperationState::Intent);
        assert_eq!(storage[0].hash.len(), 64);
        assert!(journal.has_pending_operations());
    }

    #[test]
    fn missing_original_is_null_not_empty_string() {
        let journal = load("journal/07-no-original.json");
        assert!(!journal.original_existed);
        assert_eq!(journal.original_hash, None);
        assert!(journal.operations.is_empty());
    }

    #[test]
    fn restore_sequence_orders_by_kind_then_reverses_within_kind() {
        let mut journal = load("journal/01-new.json");
        journal.operations = vec![
            Operation::new(OperationKind::Link, "link-1", OperationState::Done),
            Operation::new(OperationKind::EngineLink, "engine", OperationState::Done),
            Operation::new(OperationKind::Backup, "backup", OperationState::Done),
            Operation::new(OperationKind::Link, "link-2", OperationState::Done),
            Operation::new(OperationKind::Runtime, "runtime", OperationState::Done),
            Operation::new(OperationKind::Link, "link-3", OperationState::Done),
        ];
        let ordered: Vec<&str> = journal
            .restore_sequence()
            .iter()
            .map(|operation| operation.path.as_str())
            .collect();
        assert_eq!(
            ordered,
            vec!["engine", "backup", "runtime", "link-3", "link-2", "link-1"]
        );
    }

    #[test]
    fn round_trip_keeps_every_field() {
        for relative in [
            "journal/01-new.json",
            "journal/02-runtime-done.json",
            "journal/03-engine-link-intent.json",
            "journal/04-partial-failure.json",
            "journal/07-no-original.json",
        ] {
            let journal = load(relative);
            let text = journal.to_json().expect("序列化");
            let reparsed = Journal::from_json(&text)
                .unwrap_or_else(|error| panic!("{relative} 往返失败：{error}"));
            assert_eq!(reparsed, journal, "{relative}");
        }
    }

    #[test]
    fn empty_target_and_hash_default_when_absent() {
        let text = r#"{
            "Version": 1, "GameId": "g", "Session": "0123456789abcdef0123456789abcdef",
            "ScriptPid": 1, "ScriptStarted": "2026-11-22T00:00:00.0000000Z",
            "Render": "R", "Engine": "E", "Runtime": "T", "Backup": "B",
            "OriginalExisted": false, "OriginalHash": null, "StorageExisted": false,
            "Phase": "deploying",
            "Operations": [{"Kind": "link", "Path": "p", "State": "done"}]
        }"#;
        let journal = Journal::from_json(text).expect("解析");
        assert_eq!(journal.operations[0].target, "");
        assert_eq!(journal.operations[0].hash, "");
        assert!(journal.reused.is_empty());
        assert!(journal.proxy_name.is_none());
    }

    #[test]
    fn unknown_fields_are_tolerated_but_versions_are_not() {
        let text = r#"{
            "Version": 1, "GameId": "g", "Session": "0123456789abcdef0123456789abcdef",
            "ScriptPid": 1, "ScriptStarted": "2026-11-22T00:00:00.0000000Z",
            "Render": "R", "Engine": "E", "Runtime": "T", "Backup": "B",
            "OriginalExisted": false, "OriginalHash": null, "StorageExisted": false,
            "Phase": "deploying", "Operations": [], "Reused": [], "FutureField": 42
        }"#;
        assert!(Journal::from_json(text).is_ok());

        let older = text.replace("\"Version\": 1", "\"Version\": 2");
        let error = Journal::from_json(&older).expect_err("应拒绝新版本");
        assert!(error.to_string().contains("不支持的"), "实际：{error}");
    }

    #[test]
    fn invalid_session_is_rejected() {
        let mut journal = load("journal/01-new.json");
        journal.session = "not-a-guid".to_owned();
        let error = journal.validate().expect_err("应拒绝");
        assert!(error.to_string().contains("会话标识"), "实际：{error}");
        assert!(is_valid_session("0123456789ABCDEF0123456789abcdef"));
        assert!(!is_valid_session("0123456789abcdef0123456789abcde"));
    }

    #[test]
    fn dlssg_only_mode_round_trips() {
        let mut journal = load("journal/07-no-original.json");
        journal.deployment_mode = Some(DeploymentMode::DlssgOnly);
        journal.proxy_name = Some("winmm.dll".to_owned());
        let text = journal.to_json().expect("序列化");
        assert!(text.contains("\"DeploymentMode\": \"DlssgOnly\""), "{text}");
        assert!(text.contains("\"ProxyName\": \"winmm.dll\""), "{text}");
        let reparsed = Journal::from_json(&text).expect("解析");
        assert!(reparsed.is_dlssg_only());
        assert_eq!(reparsed.proxy_name.as_deref(), Some("winmm.dll"));
    }
}

#[cfg(test)]
mod initialize_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn initialize_builds_a_valid_deploying_record() {
        let journal = Journal::initialize(
            "wuthering",
            "0123456789abcdef0123456789abcdef",
            4242,
            "2026-11-23T00:00:00.0000000Z".to_owned(),
            Path::new(r"D:\Game\bin"),
            Path::new(r"D:\Game\bin\Engine.ini"),
            Path::new(r"C:\Temp\rt.ini"),
            Path::new(r"C:\Temp\bk.ini"),
            true,
            Some("ABCD".to_owned()),
            false,
            None,
            "version.dll",
        );
        assert_eq!(journal.version, 1);
        assert_eq!(journal.phase, Phase::Deploying);
        assert!(journal.operations.is_empty());
        assert!(journal.reused.is_empty());
        assert_eq!(journal.proxy_name.as_deref(), Some("version.dll"));
        journal.validate().expect("新记录应通过校验");
        // 序列化键名与 PS 一致
        let json = serde_json::to_string(&journal).expect("序列化");
        for key in [
            "\"GameId\"",
            "\"Session\"",
            "\"ScriptStarted\"",
            "\"Phase\":\"deploying\"",
            "\"ProxyName\"",
        ] {
            assert!(json.contains(key), "缺少 {key}：{json}");
        }
    }

    #[test]
    fn initialize_rejects_a_bad_session_through_validate() {
        let journal = Journal::initialize(
            "wuthering",
            "not-hex",
            1,
            "2026-11-23T00:00:00.0000000Z".to_owned(),
            Path::new(r"D:\Game"),
            Path::new(r"D:\Game\Engine.ini"),
            Path::new(r"C:\Temp\rt.ini"),
            Path::new(r"C:\Temp\bk.ini"),
            false,
            None,
            false,
            None,
            "version.dll",
        );
        assert!(journal.validate().is_err(), "非法会话号必须被校验拒绝");
    }
}
