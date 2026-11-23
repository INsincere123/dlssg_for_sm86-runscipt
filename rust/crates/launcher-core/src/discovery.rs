//! 本地安装发现与渲染程序候选（M3-03）。
//!
//! 对齐 `启动脚本/scripts/common/GameDiscovery.ps1` 的 `Get-GameRendererCandidates`、
//! `Get-TrustedGameRenderer`、`Test-GameX64Executable`、`Test-GameAuxiliaryExecutable`：
//!
//! - 广度优先扫描游戏目录，**最多 12000 个目录**、**最多 12 层**（超限记 issue 而不是静默截断）
//! - 跳过固定目录名（windows / redist / 第三方 / 工具 / 编辑器 / dlssg_sm86 等）
//! - 跳过重解析点（链接）与辅助程序（崩溃上报、反作弊、卸载器、Steam 助手…）
//! - 只收 **x64 PE 可执行文件**（`MZ` + `PE\0\0` + machine 0x8664 + 可执行位 + 非 DLL）
//! - 打分：同目录帧生成组件 80、UE `-Win64-Shipping.exe` 在 `\Binaries\Win64` 60、
//!   Unity 程序与 `_Data` 对应 60、平台声明入口 30、超分组件 10、DX12 命名线索 5
//! - 结果按分数降序、相对路径升序；"可信"要求 ≥60 且严格高于第二名
//!
//! 遍历可随时取消：`cancel` 每进入一个目录、每个条目都会被调用一次。

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// 扫描预算。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiscoveryLimits {
    pub max_directories: usize,
    pub max_depth: usize,
}

impl Default for DiscoveryLimits {
    fn default() -> Self {
        Self {
            max_directories: 12_000,
            max_depth: 12,
        }
    }
}

/// 一个候选渲染程序。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererCandidate {
    pub path: PathBuf,
    pub relative: String,
    pub score: i32,
    pub reasons: Vec<String>,
}

impl RendererCandidate {
    pub fn display(&self) -> String {
        format!("{} — {}", self.relative, self.reasons.join("；"))
    }
}

/// 一个发现来源（注册表记录、玩家手选目录等）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryRoot {
    /// 来源标签，例如 `Steam` / `Epic` / `GOG` / `手动`。
    pub label: String,
    pub path: PathBuf,
}

/// 合并多个来源：按路径去重（大小写不敏感），保留首次出现的标签。
pub fn merge_roots(roots: Vec<DiscoveryRoot>) -> Vec<DiscoveryRoot> {
    let mut merged: Vec<DiscoveryRoot> = Vec::new();
    for root in roots {
        let key = |path: &Path| {
            path.to_string_lossy()
                .replace('/', "\\")
                .trim_end_matches('\\')
                .to_lowercase()
        };
        let duplicate = merged
            .iter()
            .any(|existing| key(&existing.path) == key(&root.path));
        if !duplicate {
            merged.push(root);
        }
    }
    merged
}

/// 单一来源的扫描结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootReport {
    pub label: String,
    pub outcome: DiscoveryOutcome,
}

/// 逐来源扫描；不存在的来源记 issue 并跳过，不影响其它来源。
pub fn discover(
    roots: &[DiscoveryRoot],
    declared_executable: Option<&Path>,
    limits: DiscoveryLimits,
    cancel: &dyn Fn() -> bool,
) -> Vec<RootReport> {
    let mut reports = Vec::new();
    for root in merge_roots(roots.to_vec()) {
        if cancel() {
            break;
        }
        let outcome = match renderer_candidates(&root.path, declared_executable, limits, cancel) {
            Ok(outcome) => outcome,
            Err(error) => DiscoveryOutcome {
                issues: vec![format!("{}：{error}", root.label)],
                ..DiscoveryOutcome::default()
            },
        };
        reports.push(RootReport {
            label: root.label,
            outcome,
        });
    }
    reports
}

/// 扫描结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiscoveryOutcome {
    /// 已排好序（分数降序、相对路径升序）。
    pub candidates: Vec<RendererCandidate>,
    pub issues: Vec<String>,
    pub cancelled: bool,
    pub visited: usize,
}

/// 扫描游戏目录，返回候选渲染程序。
pub fn renderer_candidates(
    game_directory: &Path,
    declared_executable: Option<&Path>,
    limits: DiscoveryLimits,
    cancel: &dyn Fn() -> bool,
) -> Result<DiscoveryOutcome> {
    if !game_directory.is_dir() {
        return Err(Error::invalid(format!(
            "游戏目录必须是普通文件夹：{}",
            game_directory.display()
        )));
    }
    let root = game_directory
        .to_str()
        .ok_or_else(|| Error::invalid("游戏目录不是合法 UTF-8 路径。"))?
        .trim_end_matches('\\')
        .to_owned();

    let mut outcome = DiscoveryOutcome::default();
    let mut pending: VecDeque<(PathBuf, usize)> = VecDeque::new();
    pending.push_back((PathBuf::from(&root), 0));

    while let Some((directory, depth)) = pending.pop_front() {
        if cancel() {
            outcome.cancelled = true;
            break;
        }
        if outcome.visited >= limits.max_directories {
            outcome.issues.push(format!("扫描目录达到上限：{root}"));
            break;
        }
        outcome.visited += 1;

        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => {
                outcome
                    .issues
                    .push(format!("无法读取：{}", directory.display()));
                continue;
            }
        };
        let mut items: Vec<(PathBuf, bool)> = Vec::new();
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let path = entry.path();
            let metadata = match std::fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            items.push((path, metadata.is_dir()));
        }
        let lower_names: Vec<String> = items
            .iter()
            .filter_map(|(path, _)| path.file_name())
            .map(|name| name.to_string_lossy().to_lowercase())
            .collect();

        for (path, is_dir) in items {
            if cancel() {
                outcome.cancelled = true;
                break;
            }
            let name = path
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_default();
            if is_dir {
                if depth < limits.max_depth && !skip_directory(&name) {
                    pending.push_back((path, depth + 1));
                } else if depth >= limits.max_depth {
                    outcome
                        .issues
                        .push(format!("达到扫描深度限制：{}", path.display()));
                }
                continue;
            }
            let extension_ok = path
                .extension()
                .and_then(|value| value.to_str())
                .map(|value| value.eq_ignore_ascii_case("exe"))
                .unwrap_or(false);
            if !extension_ok || is_auxiliary_executable(&name) || !is_x64_executable(&path) {
                continue;
            }

            let directory_text = directory.to_string_lossy().to_lowercase();
            let mut score: i32 = 0;
            let mut reasons: Vec<String> = Vec::new();
            if lower_names.iter().any(|item| item == "nvngx_dlssg.dll")
                || lower_names.iter().any(|item| item == "sl.dlss_g.dll")
            {
                score += 80;
                reasons.push("同目录有帧生成组件".to_owned());
            }
            if name.to_lowercase().ends_with("-win64-shipping.exe")
                && directory_text.ends_with("\\binaries\\win64")
            {
                score += 60;
                reasons.push("UE Shipping 游戏目录".to_owned());
            }
            if lower_names.iter().any(|item| item == "unityplayer.dll")
                && lower_names
                    .iter()
                    .any(|item| item == &format!("{}_data", stem(&name).to_lowercase()))
            {
                score += 60;
                reasons.push("Unity 程序与数据目录对应".to_owned());
            }
            if let Some(declared) = declared_executable {
                // 统一分隔符再做大小写不敏感比较（PS 用 -ieq 比较完整路径）。
                let candidate = path.to_string_lossy().replace('/', "\\");
                let wanted = declared.to_string_lossy().replace('/', "\\");
                if candidate.eq_ignore_ascii_case(&wanted) {
                    score += 30;
                    reasons.push("平台声明的启动入口（需核实渲染程序）".to_owned());
                }
            }
            if lower_names.iter().any(|item| item == "nvngx_dlss.dll") {
                score += 10;
                reasons.push("有超分组件，不代表支持帧生成".to_owned());
            }
            if stem(&name).to_lowercase().contains("dx12")
                || stem(&name).to_lowercase().contains("d3d12")
                || directory_text.ends_with("\\x64_dx12")
            {
                score += 5;
                reasons.push("DX12 命名线索".to_owned());
            }
            if reasons.is_empty() {
                reasons.push("x64 程序；请手动核实游戏本体".to_owned());
            }
            let relative = path
                .strip_prefix(&root)
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_else(|_| path.to_string_lossy().into_owned());
            outcome.candidates.push(RendererCandidate {
                path,
                relative,
                score,
                reasons,
            });
        }
        if outcome.cancelled {
            break;
        }
    }

    outcome.candidates.sort_by(|left, right| {
        right.score.cmp(&left.score).then_with(|| {
            left.relative
                .to_lowercase()
                .cmp(&right.relative.to_lowercase())
        })
    });
    Ok(outcome)
}

/// `Get-TrustedGameRenderer`：分数 ≥60 且严格高于第二名才算可信。
pub fn trusted_renderer(candidates: &[RendererCandidate]) -> Option<&RendererCandidate> {
    let first = candidates.first()?;
    if first.score < 60 {
        return None;
    }
    match candidates.get(1) {
        None => Some(first),
        Some(second) if first.score > second.score => Some(first),
        Some(_) => None,
    }
}

/// 扫描时要跳过的目录名。
pub fn skip_directory(name: &str) -> bool {
    const EXACT: &[&str] = &[
        "windows",
        "$recycle.bin",
        "system volume information",
        "node_modules",
        ".git",
        "_commonredist",
        "redist",
        "redistributables",
        "prerequisites",
        "thirdparty",
        "support",
        "__installer",
        "editors",
        "sdk",
        "dlssg_sm86",
        "tool",
        "tools",
        // 启动器/补丁缓存与资源包：只在里面翻到 .pak/.utoc，没有可执行文件
        "userdata",
        "patcher",
        "patch",
        "tmp",
        "cache",
        "paks",
        "movies",
        "content",
    ];
    let lower = name.to_lowercase();
    EXACT.contains(&lower.as_str())
}

/// 辅助程序（崩溃上报、反作弊、卸载器、Steam 助手…）不算游戏本体。
pub fn is_auxiliary_executable(name: &str) -> bool {
    let role = stem(name).to_lowercase();
    let role = strip_suffix_ci(&role, "-shipping");
    let role = strip_suffix_ci(&role, "-development");
    let role = strip_suffix_ci(&role, "-debuggame");
    let role = strip_suffix_ci(&role, "-debug");
    let role = strip_suffix_ci(&role, "-test");
    let role = strip_suffix_ci(&role, "-win64");
    let role = strip_suffix_ci(&role, "-win32");
    let role = strip_suffix_ci(&role, "-x64");
    let role = strip_suffix_ci(&role, "-x86");

    const EXACT: &[&str] = &[
        "install",
        "uninstall",
        "vulkaninfo",
        "nvidia-smi",
        "d3d12stateobjectcompiler",
        "d3d12warpcompiler",
        "shadercompileworker",
        "unrealpak",
        "ue4editor",
        "unrealeditor",
        "start_protected_game",
        "gameserver",
    ];
    if EXACT.contains(&role.as_str()) {
        return true;
    }
    for prefix in [
        "crashreportclient",
        "unitycrashhandler",
        "crashpad_handler",
        "crashsender",
        "unins",
        "setup",
        "easyanticheat",
        "beservice",
        "steamwebhelper",
        "steamservice",
        "steamerrorreporter",
        "cefsubprocess",
        "qtwebengineprocess",
        "srcds",
        "vulkaninfo",
    ] {
        if role.starts_with(prefix) {
            return true;
        }
    }
    const SINGLE: [&str; 7] = [
        "launcher",
        "bootstrap",
        "bootstrapper",
        "updater",
        "helper",
        "crashreporter",
        "dedicatedserver",
    ];
    if SINGLE.contains(&role.as_str()) {
        return true;
    }
    // 边界敏感：`-launcher` / `_helper` / 空格分隔 / 驼峰 `SomeLauncher` 都算，
    // 但 `Observer`、`Crash` 这类正常标题不算。
    const PASCAL: [&str; 7] = [
        "Launcher",
        "Bootstrap",
        "Bootstrapper",
        "Updater",
        "Helper",
        "CrashReporter",
        "DedicatedServer",
    ];
    let original = stem(name);
    for suffix in PASCAL {
        if !original.ends_with(suffix) {
            continue;
        }
        let prefix = &original[..original.len() - suffix.len()];
        let boundary = prefix.is_empty()
            || prefix.ends_with('-')
            || prefix.ends_with('_')
            || prefix.ends_with(' ')
            || prefix
                .chars()
                .last()
                .map(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
                .unwrap_or(false);
        if boundary {
            return true;
        }
    }
    false
}

fn strip_suffix_ci(value: &str, suffix: &str) -> String {
    if value.len() >= suffix.len()
        && value[value.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
    {
        value[..value.len() - suffix.len()].to_owned()
    } else {
        value.to_owned()
    }
}

fn stem(name: &str) -> String {
    match name.rfind('.') {
        Some(index) if index > 0 => name[..index].to_owned(),
        _ => name.to_owned(),
    }
}

/// `Test-GameX64Executable`：只认 x64、可执行、非 DLL 的 PE。
pub fn is_x64_executable(path: &Path) -> bool {
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(_) => return false,
    };
    if data.len() < 64 || data[0] != b'M' || data[1] != b'Z' {
        return false;
    }
    let offset = i32::from_le_bytes([data[60], data[61], data[62], data[63]]);
    if offset < 64 || offset as usize > data.len().saturating_sub(26) {
        return false;
    }
    let offset = offset as usize;
    if &data[offset..offset + 4] != b"PE\0\0" {
        return false;
    }
    let machine = u16::from_le_bytes([data[offset + 4], data[offset + 5]]);
    if machine != 0x8664 {
        return false;
    }
    let flags = u16::from_le_bytes([data[offset + 22], data[offset + 23]]);
    flags & 0x0002 != 0 && flags & 0x2000 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_list_and_auxiliary_rules_match_ps() {
        for name in [
            "Windows",
            "tools",
            "tool",
            "Redist",
            "dlssg_sm86",
            ".git",
            "UserData",
            "Patcher",
            "tmp",
            "Content",
        ] {
            assert!(skip_directory(name), "应跳过 {name}");
        }
        for name in ["Binaries", "Game", "Engine"] {
            assert!(!skip_directory(name), "不应跳过 {name}");
        }
        for name in [
            "CrashReportClient.exe",
            "UnityCrashHandler64.exe",
            "Unins000.exe",
            "EasyAntiCheat.exe",
            "start_protected_game.exe",
            "SomeLauncher.exe",
            "Game-Bootstrap.exe",
            "Updater.exe",
        ] {
            assert!(is_auxiliary_executable(name), "应判为辅助程序：{name}");
        }
        for name in [
            "Observer.exe",
            "CrashBandicoot.exe",
            "Game.exe",
            "BlackMythWukong.exe",
            // UE 游戏本体不是辅助程序：PS 里 -Shipping 只是用来算 role，不参与排除
            "Game-Win64-Shipping.exe",
        ] {
            assert!(!is_auxiliary_executable(name), "不应判为辅助程序：{name}");
        }
    }

    #[test]
    fn trusted_renderer_requires_a_strict_lead() {
        let candidate = |score: i32, relative: &str| RendererCandidate {
            path: PathBuf::from(relative),
            relative: relative.to_owned(),
            score,
            reasons: Vec::new(),
        };
        assert!(trusted_renderer(&[]).is_none());
        assert!(trusted_renderer(&[candidate(59, "a.exe")]).is_none());
        assert_eq!(
            trusted_renderer(&[candidate(60, "a.exe")]).map(|item| item.relative.clone()),
            Some("a.exe".to_owned())
        );
        assert!(trusted_renderer(&[candidate(60, "a.exe"), candidate(60, "b.exe")]).is_none());
        assert_eq!(
            trusted_renderer(&[candidate(90, "a.exe"), candidate(60, "b.exe")])
                .map(|item| item.relative.clone()),
            Some("a.exe".to_owned())
        );
    }
}
