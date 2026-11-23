//! 切换显卡名称：窗口模型 + 提权子进程入口（`--gpu`）。
//!
//! 这个功能的高风险部分在 `launcher-core::gpu`（格式与判定）和 `launcher-win::gpu`
//! （注册表读写）里；本模块只做三件事：
//!
//! 1. **执行**：`--gpu apply|restore|reset-state|status`，把结果写成 JSON 报告交给界面
//!    （子进程是 `windows` 子系统、提权后没有控制台，只能靠报告沟通）
//! 2. **窗口模型**：读一次设备事实（设备 / 驱动原始名 / 备份）算出按钮可用性，便于单测
//! 3. **重启**：`shutdown /r /t <秒>`，不做任何重启检测——本次运行里改过就由界面提示
//!
//! 界面本身不写注册表：写入一律由 `--gpu` 提权子进程完成。

use std::path::{Path, PathBuf};

use launcher_core::gpu::{self, DEFAULT_TARGET_NAME, GpuSituation, GpuState};
use serde::{Deserialize, Serialize};

/// 一次显卡名称操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// 改名为目标名称（先写备份，再写注册表）。
    Apply,
    /// 还原原始名称（有备份按备份，没备份按驱动 `DriverDesc`）。
    Restore,
    /// 清除旧备份（PS 的 `-ResetState`：当前不是伪装名时才允许）。
    ResetState,
    /// 只读状态（给命令行排查用）。
    Status,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Apply => "apply",
            Self::Restore => "restore",
            Self::ResetState => "reset-state",
            Self::Status => "status",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "apply" => Some(Self::Apply),
            "restore" => Some(Self::Restore),
            "reset-state" => Some(Self::ResetState),
            "status" => Some(Self::Status),
            _ => None,
        }
    }

    /// 报告文件名（界面按它轮询结果）。
    pub fn report_name(self) -> &'static str {
        "ui-gpu.json"
    }
}

/// 提权子进程写给界面的报告。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub action: String,
    pub ok: bool,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub message: String,
}

impl Report {
    pub fn success(action: Action, message: impl Into<String>) -> Self {
        Self {
            action: action.as_str().to_owned(),
            ok: true,
            error: String::new(),
            message: message.into(),
        }
    }

    pub fn failure(action: Action, error: impl Into<String>) -> Self {
        Self {
            action: action.as_str().to_owned(),
            ok: false,
            error: error.into(),
            message: String::new(),
        }
    }

    pub fn to_json(&self) -> String {
        let mut text = serde_json::to_string_pretty(self)
            .unwrap_or_else(|_| "{\"action\":\"unknown\",\"ok\":false}".to_owned());
        text.push('\n');
        text
    }

    pub fn parse(text: &str) -> Option<Self> {
        serde_json::from_str(text.trim()).ok()
    }
}

/// 拼提权子进程的命令行（引号在这里统一加；名字已由 `validate_target_name` 排除双引号）。
pub fn arguments(action: Action, name: Option<&str>, report: &Path) -> String {
    let mut text = format!("--gpu {}", action.as_str());
    if let Some(name) = name {
        text.push_str(&format!(" --name \"{name}\""));
    }
    text.push_str(&format!(" --report \"{}\"", report.display()));
    text
}

/// 当前时间（ISO-8601 UTC），写进备份的 `CreatedAt`，只给人看。
fn now_iso8601() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    gpu::format_iso8601_utc(seconds)
}

/// 读一次设备事实：唯一 NVIDIA 显卡 + 驱动原始名 + 备份。
fn load_situation(typed_target: &str) -> Result<GpuSituation, String> {
    let devices = launcher_win::gpu::enumerate_devices();
    let device = gpu::select_device(&devices, None)
        .map_err(|error| error.to_string())?
        .clone();
    let driver = launcher_win::gpu::driver_desc(&device);
    let backup = launcher_win::gpu::load_state();
    Ok(GpuSituation::build(
        device,
        driver.as_deref(),
        backup,
        typed_target,
    ))
}

/// `--gpu` 的实现：成功与失败都返回一份报告（失败时 `ok=false`）。
pub fn execute(action: Action, name: Option<&str>) -> Report {
    match run(action, name) {
        Ok(message) => Report::success(action, message),
        Err(error) => Report::failure(action, error),
    }
}

fn run(action: Action, name: Option<&str>) -> Result<String, String> {
    let requested = name.unwrap_or("").trim().to_owned();
    let situation = load_situation(&requested)?;
    let device = situation.device.clone();
    match action {
        Action::Status => Ok(status_message(&situation)),
        Action::Apply => {
            let target = gpu::validate_target_name(name.unwrap_or(DEFAULT_TARGET_NAME))
                .map_err(|error| error.to_string())?;
            if situation.backup.is_some() {
                return Err(if situation.backup_foreign {
                    "已有状态备份，而且属于另一台设备：先点「清除旧备份」，再切换。".to_owned()
                } else {
                    "已有状态备份：请先还原原始名称，或在驱动更新恢复真名后点「清除旧备份」。"
                        .to_owned()
                });
            }
            if situation.masked {
                return Err(format!(
                    "当前已经是伪装名称（{}），不能重复切换。",
                    situation.current_name
                ));
            }
            if !situation.can_apply(&target) {
                return Err("当前名称与目标相同，无需切换。".to_owned());
            }
            let state = GpuState::capture(&device, &target, now_iso8601())
                .map_err(|error| error.to_string())?;
            let saved =
                launcher_win::gpu::save_state(&state, false).map_err(|error| error.to_string())?;
            if let Err(error) =
                launcher_win::gpu::write_device_desc(&device.instance_id, &state.expected_value)
            {
                return Err(format!(
                    "{error}；状态备份已保留在 {}，可用「恢复原名称」或 PS 的切换显卡.bat 还原。",
                    saved.display()
                ));
            }
            Ok(format!(
                "已切换为 {target}；需要重启电脑才生效（重启过可忽略提示）。"
            ))
        }
        Action::Restore => {
            if !situation.masked {
                return Err("当前不是伪装名称，无需还原。".to_owned());
            }
            let value = match situation.owned_backup() {
                Some(state) => state
                    .restore_value(&device.device_desc)
                    .map_err(|error| error.to_string())?,
                None => gpu::value_from_driver_desc(
                    &device.device_desc,
                    situation.original_name.as_deref().unwrap_or(""),
                )
                .map_err(|error| {
                    if situation.backup_foreign {
                        format!(
                            "{error}（状态备份属于另一台设备：先点「清除旧备份」，或用 PS 的切换显卡.bat）"
                        )
                    } else {
                        format!("{error}（可用 PS 的切换显卡.bat 还原）")
                    }
                })?,
            };
            launcher_win::gpu::write_device_desc(&device.instance_id, &value)
                .map_err(|error| error.to_string())?;
            // 还原成功后备份就没用了；与 PS 一样删掉，避免"旧备份残留"。
            let _ = launcher_win::gpu::delete_state();
            Ok(format!(
                "已还原为 {}；需要重启电脑才生效（重启过可忽略提示）。",
                situation.original_name.as_deref().unwrap_or("原始名称")
            ))
        }
        Action::ResetState => {
            if situation.masked {
                return Err("当前仍是伪装名称，不能清除备份；请先还原原始名称。".to_owned());
            }
            if situation.backup.is_none() {
                return Ok("没有旧备份需要清除。".to_owned());
            }
            launcher_win::gpu::delete_state().map_err(|error| error.to_string())?;
            Ok("已清除旧的状态备份。".to_owned())
        }
    }
}

/// `--gpu status` 的一行摘要（命令行排查用）。
fn status_message(situation: &GpuSituation) -> String {
    format!(
        "{}（{}）；当前 {}；原始 {}；{}；注册表可写：{}",
        situation.device.display_name,
        situation.device.instance_id,
        situation.current_name,
        situation.original_name.as_deref().unwrap_or("未知"),
        match (&situation.backup, situation.masked) {
            (Some(_), true) => "已伪装，有备份",
            (Some(_), false) => "原始名称，有旧备份",
            (None, true) => "已伪装，无备份",
            (None, false) => "原始名称，无备份",
        },
        if launcher_win::gpu::writable(&situation.device) {
            "是"
        } else {
            "否（需要管理员，或键的 ACL 不允许写）"
        }
    )
}

/// 本次运行里成功改过什么：决定窗口里的红字提示（切换和还原都提示重启）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Switched,
    Restored,
}

impl Change {
    pub fn text(self) -> &'static str {
        match self {
            Self::Switched => "本次已修改显卡名称，重启电脑后生效",
            Self::Restored => "本次已还原显卡名称，重启电脑后生效",
        }
    }

    /// 由刚完成的动作推出要提示什么；`status`/`reset-state` 不动名称，不提示。
    pub fn from_action(action: Action) -> Option<Self> {
        match action {
            Action::Apply => Some(Self::Switched),
            Action::Restore => Some(Self::Restored),
            Action::ResetState | Action::Status => None,
        }
    }
}

/// 本次运行里的改名轨迹：只有"和本次运行开始时的名字不同"才需要重启。
///
/// 先切换再还原回去 = 净改动为零，不提示重启（真机要求 2026-11-23）。
/// 基线只在**第一次成功改动之前**记一次，之后一直比它。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionChanges {
    /// 本次运行第一次成功改动之前的显示名。
    baseline: Option<String>,
}

impl SessionChanges {
    /// 动作开始前记录基线（只记第一次；动作失败也不影响，因为名字没变）。
    pub fn remember_baseline(&mut self, current: &str) {
        let current = current.trim();
        if self.baseline.is_none() && !current.is_empty() {
            self.baseline = Some(current.to_owned());
        }
    }

    /// 动作成功后结算：名字回到基线 → 不需要重启提示；否则按动作给提示。
    pub fn settle(&mut self, current: &str, action: Action) -> Option<Change> {
        let baseline = self.baseline.as_deref()?;
        if baseline.eq_ignore_ascii_case(current.trim()) {
            self.baseline = None;
            return None;
        }
        Change::from_action(action)
    }
}

/// 打开窗口时输入框里先填什么。
///
/// 优先用"现役目标"（已伪装时备份里的 `TargetName`，可能是 PS 工具设的），
/// 其次用本机记住的上次成功名字，最后才是默认 5070。
pub fn initial_target(backup: Option<&GpuState>, masked: bool, remembered: &str) -> String {
    if masked {
        if let Some(name) = backup
            .map(|state| state.target_name.trim())
            .filter(|name| !name.is_empty())
        {
            return name.to_owned();
        }
    }
    let remembered = remembered.trim();
    if remembered.is_empty() {
        DEFAULT_TARGET_NAME.to_owned()
    } else {
        remembered.to_owned()
    }
}

/// 窗口状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Panel {
    /// 输入框里的目标名。
    pub target: String,
    /// `Err` 表示读不到（没有 NVIDIA 显卡 / 多张 / 注册表读不了）。
    pub situation: Result<GpuSituation, String>,
    /// 动作失败的说明（窗口内红字）。
    pub error: String,
    /// 正在等提权子进程。
    pub busy: bool,
    /// 正在二次确认重启。
    pub confirm_restart: bool,
    /// 已经排上重启（`shutdown /r`），系统倒计时里就别再点了。
    pub restart_sent: bool,
}

impl Panel {
    /// 读一次事实并决定初始目标名。
    pub fn load(remembered: &str) -> Self {
        let mut panel = Self {
            target: remembered.to_owned(),
            situation: Err(String::new()),
            error: String::new(),
            busy: false,
            confirm_restart: false,
            restart_sent: false,
        };
        panel.refresh();
        panel
    }

    /// 重新读设备事实（动作结束后、点「刷新」时）。
    ///
    /// 已伪装时目标名跟着"现役目标"走（可能是 PS 工具设的名字）；否则保留用户正在编辑的值，
    /// 只有空着才填默认的 5070。
    pub fn refresh(&mut self) {
        match load_situation(&self.target) {
            Ok(situation) => {
                if situation.masked {
                    self.target = initial_target(situation.backup.as_ref(), true, &self.target);
                } else if self.target.trim().is_empty() {
                    self.target = initial_target(None, false, "");
                }
                self.situation = Ok(situation);
            }
            Err(error) => self.situation = Err(error),
        }
    }

    pub fn can_apply(&self) -> bool {
        match &self.situation {
            Ok(situation) => {
                !self.busy
                    && gpu::validate_target_name(&self.target).is_ok()
                    && situation.can_apply(&self.target)
            }
            Err(_) => false,
        }
    }

    pub fn can_restore(&self) -> bool {
        matches!(&self.situation, Ok(situation) if !self.busy && situation.can_restore())
    }

    pub fn can_reset(&self) -> bool {
        matches!(&self.situation, Ok(situation) if !self.busy && situation.can_reset())
    }

    pub fn can_restart(&self) -> bool {
        !self.busy && !self.restart_sent && self.situation.is_ok()
    }

    /// 状态行文案。
    pub fn status_text(&self) -> String {
        match &self.situation {
            Err(error) if error.is_empty() => "正在读取显卡信息…".to_owned(),
            Err(error) => error.clone(),
            Ok(situation) => {
                let mut text = format!("当前：{}", situation.current_name);
                text.push_str(if situation.masked {
                    "　状态：已切换"
                } else {
                    "　状态：原始名称（未切换）"
                });
                match &situation.backup {
                    Some(state) if situation.backup_foreign => {
                        text.push_str("　备份：属于另一台设备（只能清除）");
                    }
                    Some(state) => {
                        text.push_str(&format!(
                            "　备份：原值 → {}",
                            state
                                .original_display_name()
                                .unwrap_or_else(|_| "（无法解析）".to_owned())
                        ));
                    }
                    None => text.push_str("　备份：无"),
                }
                text
            }
        }
    }
}

/// `shutdown.exe` 的绝对路径。
pub fn shutdown_executable() -> PathBuf {
    let root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    root.join("System32").join("shutdown.exe")
}

/// 让系统在 `seconds` 秒后重启；返回对话框里要显示的说明。
pub fn restart(seconds: u32) -> Result<String, String> {
    let executable = shutdown_executable();
    if !executable.is_file() {
        return Err(format!("找不到 {}", executable.display()));
    }
    let arguments = vec![
        "/r".to_owned(),
        "/t".to_owned(),
        seconds.to_string(),
        "/c".to_owned(),
        "DLSSG 启动器：显卡名称切换需要重启".to_owned(),
    ];
    launcher_win::spawn_process(&executable, &arguments, None, true)
        .map_err(|error| format!("启动重启失败：{error}"))?;
    Ok(format!(
        "{seconds} 秒后重启；要取消就在命令行运行 shutdown /a"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use launcher_core::gpu::DeviceInfo;

    fn device(instance: &str, name: &str) -> DeviceInfo {
        DeviceInfo {
            instance_id: instance.to_owned(),
            display_name: name.to_owned(),
            device_desc: format!(r"@oem.inf,%dev%;{name}"),
        }
    }

    fn state(instance: &str, original: &str, target: &str) -> GpuState {
        GpuState {
            key_path: gpu::key_path_of(instance),
            original_value: format!(r"@oem.inf,%dev%;{original}"),
            expected_value: format!(r"@oem.inf,%dev%;{target}"),
            target_name: target.to_owned(),
            created_at: "2026-11-23T09:20:00Z".to_owned(),
        }
    }

    #[test]
    fn arguments_quote_the_name_and_report() {
        let text = arguments(
            Action::Apply,
            Some("NVIDIA GeForce RTX 5070"),
            Path::new(r"D:\root\logs\ui-gpu.json"),
        );
        assert_eq!(
            text,
            r#"--gpu apply --name "NVIDIA GeForce RTX 5070" --report "D:\root\logs\ui-gpu.json""#
        );
        let restore = arguments(Action::Restore, None, Path::new(r"C:\x\ui-gpu.json"));
        assert_eq!(restore, r#"--gpu restore --report "C:\x\ui-gpu.json""#);
    }

    #[test]
    fn actions_round_trip_through_text() {
        for action in [
            Action::Apply,
            Action::Restore,
            Action::ResetState,
            Action::Status,
        ] {
            assert_eq!(Action::parse(action.as_str()), Some(action));
        }
        assert_eq!(Action::parse("drop-table"), None);
    }

    #[test]
    fn reports_round_trip_and_keep_the_error() {
        let ok = Report::success(Action::Apply, "已切换");
        let parsed = Report::parse(&ok.to_json()).expect("解析");
        assert!(parsed.ok);
        assert_eq!(parsed.action, "apply");

        let failed = Report::failure(Action::Restore, "写入注册表失败：拒绝访问");
        let parsed = Report::parse(&failed.to_json()).expect("解析");
        assert!(!parsed.ok);
        assert!(parsed.error.contains("拒绝访问"));
        assert!(Report::parse("not json").is_none());
    }

    #[test]
    fn only_apply_and_restore_ask_for_a_restart() {
        assert_eq!(Change::from_action(Action::Apply), Some(Change::Switched));
        assert_eq!(Change::from_action(Action::Restore), Some(Change::Restored));
        assert_eq!(Change::from_action(Action::ResetState), None);
        assert_eq!(Change::from_action(Action::Status), None);
        assert!(Change::Switched.text().contains("重启"));
        assert!(Change::Restored.text().contains("重启"));
    }

    #[test]
    fn changing_back_to_the_original_name_needs_no_restart() {
        let mut changes = SessionChanges::default();
        // 运行开始时是"原始名"，记基线
        changes.remember_baseline("NVIDIA GeForce RTX 3080 Ti Laptop GPU");
        // 切换成功 → 要重启
        assert_eq!(
            changes.settle("NVIDIA GeForce RTX 5070", Action::Apply),
            Some(Change::Switched)
        );
        // 又还原回去 → 净改动为零，不提示
        assert_eq!(
            changes.settle("nvidia geforce rtx 3080 ti laptop gpu", Action::Restore),
            None,
            "改回原样不该再要求重启（大小写不敏感）"
        );
        // 基线被重置：下次再改才重新提示
        changes.remember_baseline("NVIDIA GeForce RTX 3080 Ti Laptop GPU");
        assert_eq!(
            changes.settle("NVIDIA GeForce RTX 4090", Action::Apply),
            Some(Change::Switched)
        );
    }

    #[test]
    fn restoring_a_mask_that_existed_before_this_run_still_asks_for_a_restart() {
        let mut changes = SessionChanges::default();
        // 运行开始时就已是伪装态（PS 工具留下的），基线是伪装名
        changes.remember_baseline("NVIDIA GeForce RTX 5070");
        assert_eq!(
            changes.settle("NVIDIA GeForce RTX 3080 Ti Laptop GPU", Action::Restore),
            Some(Change::Restored)
        );
    }

    #[test]
    fn baseline_is_recorded_once_and_missing_baseline_means_no_prompt() {
        let mut changes = SessionChanges::default();
        changes.remember_baseline("A");
        changes.remember_baseline("B");
        assert_eq!(
            changes.settle("C", Action::Apply),
            Some(Change::Switched),
            "基线不能被第二次改动覆盖"
        );

        let mut empty = SessionChanges::default();
        assert_eq!(empty.settle("C", Action::Apply), None);
    }

    #[test]
    fn the_target_starts_from_the_active_or_remembered_name() {
        let instance = r"PCI\VEN_10DE&DEV_2491&SUBSYS_1&4&1";
        // 第一次用：默认 5070
        assert_eq!(initial_target(None, false, ""), DEFAULT_TARGET_NAME);
        // 记住过名字：用记住的
        assert_eq!(
            initial_target(None, false, "NVIDIA GeForce RTX 4090"),
            "NVIDIA GeForce RTX 4090"
        );
        // 已伪装：用现役目标（可能是 PS 工具设的），压过记忆值
        let backup = state(instance, "RTX 3080 Ti", "NVIDIA GeForce RTX 4080");
        assert_eq!(
            initial_target(Some(&backup), true, "NVIDIA GeForce RTX 4090"),
            "NVIDIA GeForce RTX 4080"
        );
        // 未伪装但有旧备份：仍然用记忆值，不用旧备份里的目标
        assert_eq!(
            initial_target(Some(&backup), false, "NVIDIA GeForce RTX 4090"),
            "NVIDIA GeForce RTX 4090"
        );
    }

    #[test]
    fn panel_buttons_follow_the_situation() {
        let instance = r"PCI\VEN_10DE&DEV_2491&SUBSYS_1&4&1";
        let clean = GpuSituation::build(
            device(instance, "NVIDIA GeForce RTX 3080 Ti"),
            Some("NVIDIA GeForce RTX 3080 Ti"),
            None,
            "",
        );
        let mut panel = Panel {
            target: DEFAULT_TARGET_NAME.to_owned(),
            situation: Ok(clean),
            error: String::new(),
            busy: false,
            confirm_restart: false,
            restart_sent: false,
        };
        assert!(panel.can_apply());
        assert!(!panel.can_restore());
        assert!(!panel.can_reset());
        assert!(panel.status_text().contains("未切换"));

        // 忙碌时所有动作都停
        panel.busy = true;
        assert!(!panel.can_apply());
        assert!(!panel.can_restart());
        panel.busy = false;

        // 目标名非法时不能切换
        panel.target = "A;B".to_owned();
        assert!(!panel.can_apply());
        panel.target = "  NVIDIA GeForce RTX 5070  ".to_owned();
        assert!(panel.can_apply(), "首尾空白应被忽略");
    }

    #[test]
    fn panel_shows_the_loading_and_error_states() {
        let mut panel = Panel {
            target: String::new(),
            situation: Err(String::new()),
            error: String::new(),
            busy: false,
            confirm_restart: false,
            restart_sent: false,
        };
        assert!(panel.status_text().contains("正在读取"));
        assert!(!panel.can_apply());

        panel.situation = Err("无当前存在的 NVIDIA 显卡。".to_owned());
        assert!(panel.status_text().contains("无当前存在"));
        assert!(!panel.can_restore());
        assert!(!panel.can_restart());
    }

    #[test]
    fn restart_command_is_a_timed_shutdown() {
        assert!(shutdown_executable().ends_with(r"System32\shutdown.exe"));
    }

    /// 真机探针：`cargo test -p launcher gpu -- --ignored --nocapture probe_real_gpu`
    ///
    /// **只读**：枚举本机 NVIDIA 显卡、读 `%ProgramData%` 备份、算一遍窗口初始状态。
    #[test]
    #[ignore = "真机检查用：读本机 NVIDIA 显卡与状态备份（不写注册表）"]
    fn probe_real_gpu() {
        println!("status: {}", execute(Action::Status, None).to_json());
        let panel = Panel::load("");
        println!("窗口目标：{}", panel.target);
        println!("窗口状态：{}", panel.status_text());
        println!(
            "按钮：切换 {} / 还原 {} / 清旧备份 {} / 重启 {}",
            panel.can_apply(),
            panel.can_restore(),
            panel.can_reset(),
            panel.can_restart()
        );
    }
}
