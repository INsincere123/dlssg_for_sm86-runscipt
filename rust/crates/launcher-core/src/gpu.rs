//! NVIDIA 显卡设备名伪装（M5-01，纯逻辑部分）。
//!
//! 逐条对齐 `启动脚本/scripts/tools/Toggle-NvidiaDeviceName.ps1`：
//!
//! - `DeviceDesc` 形如 `@oemXX.inf,%nvidia_dev.xxxx%;NVIDIA GeForce RTX 3080 Ti Laptop GPU`：
//!   最后一个 `;` 之前是前缀（含分号），之后是显示名
//! - 没有 `;` 且非空、不以 `@` 开头 → 整个值就是显示名（前缀为空）
//! - 其余情况（以 `@` 开头却没有 `;`、或 `;` 在末尾）一律**拒绝覆盖**，避免把格式猜坏
//! - 设备筛选：实例 ID 以 `PCI\VEN_10DE&` 开头（大小写不敏感），必须唯一；
//!   本项目只面向 NVIDIA，核显天然被排除，**不支持多显卡**
//! - 状态备份写到 `%ProgramData%\Toggle-NvidiaDeviceName\state.json`，字段与 PS 工具**逐字段一致**
//!   （`KeyPath` / `OriginalValue` / `ExpectedValue` / `TargetName` / `CreatedAt`），
//!   这样两个工具可以互相还原对方的改动
//! - 改名与还原都需要重启才生效；本模块不做任何重启检测，也不重启电脑

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// 第一次切换时默认填的目标名（用户确认：默认就是 5070）。
pub const DEFAULT_TARGET_NAME: &str = "NVIDIA GeForce RTX 5070";

/// 目标名长度上限：名字只活在注册表与界面上，超长没有意义，还会把窗口撑坏。
pub const MAX_TARGET_NAME_CHARS: usize = 100;

/// 设备枚举根（PS 的 `$deviceKeyPath` 就是把实例 ID 拼在这后面）。
pub const ENUM_KEY_ROOT: &str = r"HKLM:\SYSTEM\CurrentControlSet\Enum";

/// 拆分后的设备描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceDescription {
    /// 前缀（含结尾分号）；纯名字时为空串。
    pub prefix: String,
    pub display_name: String,
}

impl DeviceDescription {
    /// 还原成注册表值。
    pub fn compose(&self) -> String {
        format!("{}{}", self.prefix, self.display_name)
    }

    /// 换成新的显示名。
    pub fn with_display_name(&self, name: &str) -> Self {
        Self {
            prefix: self.prefix.clone(),
            display_name: name.to_owned(),
        }
    }
}

/// 拆分 `DeviceDesc`；格式意外时返回错误（与 PS 一样拒绝覆盖）。
pub fn split_description(value: &str) -> Result<DeviceDescription> {
    let trimmed = value.trim_end_matches('\0');
    let separator = trimmed.rfind(';');
    match separator {
        None => {
            if trimmed.trim().is_empty() || trimmed.starts_with('@') {
                return Err(Error::invalid(format!(
                    "DeviceDesc has an unexpected format and will not be overwritten: {trimmed}"
                )));
            }
            Ok(DeviceDescription {
                prefix: String::new(),
                display_name: trimmed.to_owned(),
            })
        }
        Some(index) if index + 1 >= trimmed.len() => Err(Error::invalid(format!(
            "DeviceDesc has an unexpected format and will not be overwritten: {trimmed}"
        ))),
        Some(index) => Ok(DeviceDescription {
            prefix: trimmed[..index + 1].to_owned(),
            display_name: trimmed[index + 1..].to_owned(),
        }),
    }
}

/// 当前显示名是否已经是目标名（大小写不敏感）。
pub fn is_masked(value: &str, target: &str) -> bool {
    split_description(value)
        .map(|description| description.display_name.eq_ignore_ascii_case(target))
        .unwrap_or(false)
}

/// 校验用户填的目标名：返回去首尾空白后的名字。
///
/// 拒绝的东西都有理由：空名字写进去等于把设备名抹掉；`;` 会改变 `DeviceDesc` 的分号语义
/// （下次拆分就认错显示名）；`"` 会破坏提权子进程的命令行；控制字符会让注册表值不可读。
pub fn validate_target_name(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(Error::invalid("显卡名称不能为空。"));
    }
    if trimmed.contains(';') {
        return Err(Error::invalid(
            "显卡名称不能包含分号（会破坏注册表里的 DeviceDesc 格式）。",
        ));
    }
    if trimmed.contains('"') {
        return Err(Error::invalid("显卡名称不能包含双引号。"));
    }
    if trimmed.chars().any(|character| character.is_control()) {
        return Err(Error::invalid("显卡名称不能包含控制字符。"));
    }
    let length = trimmed.chars().count();
    if length > MAX_TARGET_NAME_CHARS {
        return Err(Error::invalid(format!(
            "显卡名称太长（{length} 字，最多 {MAX_TARGET_NAME_CHARS} 字）。"
        )));
    }
    Ok(trimmed.to_owned())
}

/// 一台显卡。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// `PCI\VEN_10DE&DEV_2491&SUBSYS_...\4&...`
    pub instance_id: String,
    /// 当前显示名（`DeviceDesc` 里取到的）。
    pub display_name: String,
    /// 注册表里原始的 `DeviceDesc`（写回时用）。
    pub device_desc: String,
}

impl DeviceInfo {
    pub fn is_nvidia(&self) -> bool {
        is_nvidia_instance(&self.instance_id)
    }
}

/// 实例 ID 是否是 NVIDIA 显卡（`PCI\VEN_10DE&`）。
pub fn is_nvidia_instance(instance_id: &str) -> bool {
    instance_id
        .to_ascii_uppercase()
        .starts_with(r"PCI\VEN_10DE&")
}

/// 实例 ID → PS 工具的 `KeyPath`（`HKLM:\SYSTEM\CurrentControlSet\Enum\...`）。
pub fn key_path_of(instance_id: &str) -> String {
    format!("{ENUM_KEY_ROOT}\\{}", instance_id.trim())
}

/// `KeyPath` → 实例 ID（兼容 `HKLM:` / `HKEY_LOCAL_MACHINE` 两种写法；不是注册表路径就原样返回）。
pub fn instance_from_key_path(key_path: &str) -> String {
    let trimmed = key_path.trim();
    let lowered = trimmed.to_ascii_lowercase();
    for prefix in [
        r"hklm:\system\currentcontrolset\enum\",
        r"hkey_local_machine\system\currentcontrolset\enum\",
    ] {
        if lowered.starts_with(prefix) {
            return trimmed[prefix.len()..].to_owned();
        }
    }
    trimmed.to_owned()
}

/// 选出唯一目标设备；`explicit` 给了就按它精确匹配。
pub fn select_device<'a>(
    devices: &'a [DeviceInfo],
    explicit: Option<&str>,
) -> Result<&'a DeviceInfo> {
    let candidates: Vec<&DeviceInfo> = devices.iter().filter(|item| item.is_nvidia()).collect();
    let selected: Vec<&DeviceInfo> = match explicit {
        Some(value) => candidates
            .iter()
            .copied()
            .filter(|item| item.instance_id.eq_ignore_ascii_case(value))
            .collect(),
        None => candidates.clone(),
    };
    if selected.len() == 1 {
        return Ok(selected[0]);
    }
    let mut listing = candidates
        .iter()
        .map(|item| format!("{} : {}", item.display_name, item.instance_id))
        .collect::<Vec<_>>()
        .join("\r\n");
    if listing.is_empty() {
        listing = "无当前存在的 NVIDIA 显卡。".to_owned();
    }
    Err(Error::invalid(format!(
        "无法唯一确定 NVIDIA 显卡（本项目只支持单张 NVIDIA，核显已忽略）：\r\n{listing}"
    )))
}

/// 状态备份（`%ProgramData%\Toggle-NvidiaDeviceName\state.json`）。
///
/// 字段名与 PS 工具完全一致（PascalCase）——两个工具必须能互相还原：
/// 谁写的备份，另一个都要认得。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuState {
    /// PS 的 `KeyPath`：`HKLM:\SYSTEM\CurrentControlSet\Enum\PCI\...`。
    #[serde(rename = "KeyPath", alias = "instance_id")]
    pub key_path: String,
    /// 伪装前的 `DeviceDesc` 原值。
    #[serde(rename = "OriginalValue", alias = "original_value")]
    pub original_value: String,
    /// 伪装后写进去的值（还原前核对该值，防止覆盖别人后来的改动）。
    #[serde(rename = "ExpectedValue", alias = "expected_value", default)]
    pub expected_value: String,
    #[serde(rename = "TargetName", alias = "target_name")]
    pub target_name: String,
    /// 写入时刻（ISO-8601，仅供人看；本程序不拿它做重启判断）。
    #[serde(rename = "CreatedAt", alias = "created_at", default)]
    pub created_at: String,
}

impl GpuState {
    /// 由当前设备与目标名构造备份。
    pub fn capture(device: &DeviceInfo, target: &str, created_at: String) -> Result<Self> {
        let description = split_description(&device.device_desc)?;
        let masked = description.with_display_name(target);
        Ok(Self {
            key_path: key_path_of(&device.instance_id),
            original_value: device.device_desc.clone(),
            expected_value: masked.compose(),
            target_name: target.to_owned(),
            created_at,
        })
    }

    /// 备份对应的设备实例 ID（`PCI\...`）。
    pub fn instance_id(&self) -> String {
        instance_from_key_path(&self.key_path)
    }

    /// 备份是否属于这台设备。
    pub fn belongs_to(&self, device: &DeviceInfo) -> bool {
        self.instance_id().eq_ignore_ascii_case(&device.instance_id)
    }

    /// 原值里的显示名。
    pub fn original_display_name(&self) -> Result<String> {
        Ok(split_description(&self.original_value)?.display_name)
    }

    /// 伪装后应写入的值。
    pub fn masked_value(&self) -> Result<String> {
        let description = split_description(&self.original_value)?;
        Ok(description.with_display_name(&self.target_name).compose())
    }

    /// 还原时要写回的值；当前值已被别人改过就拒绝（与 PS 同语义）。
    pub fn restore_value(&self, current_value: &str) -> Result<String> {
        if !self.expected_value.is_empty() && current_value != self.expected_value {
            return Err(Error::invalid(
                "注册表值在伪装之后被改过，拒绝覆盖陌生值（状态备份已保留）。",
            ));
        }
        Ok(self.original_value.clone())
    }
}

/// 没有备份时的兜底还原：沿用当前前缀，把显示名换成驱动自己的 `DriverDesc`。
pub fn value_from_driver_desc(current_value: &str, driver_desc: &str) -> Result<String> {
    let trimmed = driver_desc.trim();
    if trimmed.is_empty() {
        return Err(Error::invalid(
            "驱动没有可用的 DriverDesc，无法推断原始名称。",
        ));
    }
    let description = split_description(current_value)?;
    Ok(description.with_display_name(trimmed).compose())
}

/// 界面/命令行要显示的一组事实：这台设备现在是什么状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuSituation {
    pub device: DeviceInfo,
    /// 原始显示名：优先取备份里的，其次取驱动 `DriverDesc`。
    pub original_name: Option<String>,
    pub current_name: String,
    /// 当前是否处于伪装态。
    pub masked: bool,
    /// 状态备份（文件存在就有值，可能属于别的设备）。
    pub backup: Option<GpuState>,
    /// 备份属于另一台设备（换过显卡 / 驱动重装换了实例 ID）：只能清除，不能用来还原。
    pub backup_foreign: bool,
}

impl GpuSituation {
    /// 汇总事实。`typed_target` 只在"既没有备份也读不到 DriverDesc"时用来判断伪装态
    /// （退化成 PS 的 `当前名 == 目标名` 规则）。
    pub fn build(
        device: DeviceInfo,
        driver_desc: Option<&str>,
        backup: Option<GpuState>,
        typed_target: &str,
    ) -> Self {
        let backup_foreign = backup
            .as_ref()
            .map(|state| !state.belongs_to(&device))
            .unwrap_or(false);
        let owned_backup = backup
            .as_ref()
            .filter(|state| state.belongs_to(&device))
            .cloned();
        let current_name = device.display_name.clone();
        let original_name = owned_backup
            .as_ref()
            .and_then(|state| state.original_display_name().ok())
            .or_else(|| {
                driver_desc
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned)
            });
        let masked = match original_name.as_deref() {
            Some(original) => !current_name.eq_ignore_ascii_case(original),
            None => current_name.eq_ignore_ascii_case(typed_target.trim()),
        };
        Self {
            device,
            original_name,
            current_name,
            masked,
            backup,
            backup_foreign,
        }
    }

    /// 属于这台设备的备份（还原时只能用这个）。
    pub fn owned_backup(&self) -> Option<&GpuState> {
        self.backup.as_ref().filter(|_| !self.backup_foreign)
    }

    /// 能不能切换成 `target`：必须当前没被改过、没有旧备份（本机或别的设备的都不行）、
    /// 名字确实不一样。
    pub fn can_apply(&self, target: &str) -> bool {
        if self.masked || self.backup.is_some() {
            return false;
        }
        let target = target.trim();
        !target.is_empty() && !self.current_name.eq_ignore_ascii_case(target)
    }

    /// 能不能还原：当前是伪装态，且知道原始名（本机备份或 `DriverDesc`）。
    pub fn can_restore(&self) -> bool {
        self.masked && self.original_name.is_some()
    }

    /// 能不能清除旧备份（PS 的 `-ResetState`）：当前不是伪装名时才能清。
    ///
    /// 备份属于别的设备时也要能清——否则换过显卡的人就卡在"有备份、不能切、不能还原"。
    pub fn can_reset(&self) -> bool {
        self.backup.is_some() && !self.masked
    }
}

/// `%ProgramData%\Toggle-NvidiaDeviceName`。
pub const STATE_DIRECTORY_NAME: &str = "Toggle-NvidiaDeviceName";

/// 状态文件名。
pub const STATE_FILE_NAME: &str = "state.json";

/// 把 Unix 秒格式化成 `CreatedAt` 用的 ISO-8601（UTC，`Z` 结尾）。
///
/// 只做给人看的时间戳：本程序不拿它判断"是否已重启"。自己做是因为仓库不引时间库。
pub fn format_iso8601_utc(epoch_seconds: i64) -> String {
    let days = epoch_seconds.div_euclid(86_400);
    let seconds_of_day = epoch_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Howard Hinnant 的 `civil_from_days`：1970-01-01 起的天数 → 年月日。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_part + 2) / 5 + 1) as u32;
    let month = if month_part < 10 {
        month_part + 3
    } else {
        month_part - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_prefixed_description_at_the_last_semicolon() {
        let value = r"@oem12.inf,%nvidia_dev.2491%;NVIDIA GeForce RTX 3080 Ti Laptop GPU";
        let description = split_description(value).expect("拆分");
        assert_eq!(description.prefix, r"@oem12.inf,%nvidia_dev.2491%;");
        assert_eq!(
            description.display_name,
            "NVIDIA GeForce RTX 3080 Ti Laptop GPU"
        );
        assert_eq!(description.compose(), value);
    }

    #[test]
    fn plain_names_are_accepted_but_at_prefixed_ones_are_not() {
        let description = split_description("自定义显卡名").expect("纯名字");
        assert_eq!(description.prefix, "");
        assert_eq!(description.display_name, "自定义显卡名");

        for bad in [r"@oem12.inf,%nvidia_dev.2491%", "@x.inf;", "", "   "] {
            let error = split_description(bad).expect_err(bad);
            assert!(
                error.to_string().contains("unexpected format"),
                "{bad} → {error}"
            );
        }
        assert_eq!(
            split_description("a;b;c").expect("多分号").display_name,
            "c"
        );
        assert_eq!(
            split_description("value\0")
                .expect("尾部 NUL 要被忽略")
                .display_name,
            "value"
        );
    }

    #[test]
    fn masking_rewrites_only_the_display_name() {
        let value = r"@oem12.inf,%nvidia_dev.2491%;NVIDIA GeForce RTX 3080 Ti Laptop GPU";
        let description = split_description(value).expect("拆分");
        let masked = description.with_display_name("NVIDIA GeForce RTX 4090");
        assert_eq!(
            masked.compose(),
            r"@oem12.inf,%nvidia_dev.2491%;NVIDIA GeForce RTX 4090"
        );
        assert!(is_masked(&masked.compose(), "nvidia geforce rtx 4090"));
        assert!(!is_masked(value, "NVIDIA GeForce RTX 4090"));
    }

    fn device(instance: &str, name: &str) -> DeviceInfo {
        DeviceInfo {
            instance_id: instance.to_owned(),
            display_name: name.to_owned(),
            device_desc: format!(r"@oem.inf,%dev%;{name}"),
        }
    }

    fn state(instance: &str, original: &str, target: &str) -> GpuState {
        GpuState {
            key_path: key_path_of(instance),
            original_value: format!(r"@oem.inf,%dev%;{original}"),
            expected_value: format!(r"@oem.inf,%dev%;{target}"),
            target_name: target.to_owned(),
            created_at: "2026-11-23T09:20:00Z".to_owned(),
        }
    }

    #[test]
    fn only_unique_nvidia_devices_are_selected() {
        let devices = vec![
            device(
                r"PCI\VEN_10DE&DEV_2491&SUBSYS_1&4&1",
                "NVIDIA GeForce RTX 3080 Ti",
            ),
            device(r"PCI\VEN_8086&DEV_46A6&SUBSYS_1&4&2", "Intel UHD Graphics"),
        ];
        let selected = select_device(&devices, None).expect("唯一");
        assert!(selected.is_nvidia());

        let explicit =
            select_device(&devices, Some(r"PCI\VEN_10DE&DEV_2491&SUBSYS_1&4&1")).expect("显式匹配");
        assert_eq!(explicit.display_name, "NVIDIA GeForce RTX 3080 Ti");

        let error = select_device(&devices, Some(r"PCI\VEN_10DE&DEV_0000&SUBSYS_1&4&9"))
            .expect_err("不存在应报错");
        assert!(error.to_string().contains("无法唯一确定"), "{error}");
        assert!(
            error.to_string().contains("RTX 3080 Ti"),
            "应列出候选：{error}"
        );
    }

    #[test]
    fn two_nvidia_cards_are_refused_not_guessed() {
        let devices = vec![
            device(r"PCI\VEN_10DE&DEV_2491&SUBSYS_1&4&1", "RTX 3080 Ti"),
            device(r"PCI\VEN_10DE&DEV_28E0&SUBSYS_1&4&2", "RTX 4060 Laptop"),
        ];
        let error = select_device(&devices, None).expect_err("多张要拒绝");
        assert!(error.to_string().contains("只支持单张"), "{error}");
    }

    #[test]
    fn no_nvidia_device_reports_the_same_wording() {
        let devices = vec![device(r"PCI\VEN_8086&DEV_46A6&SUBSYS_1&4&2", "Intel UHD")];
        let error = select_device(&devices, None).expect_err("应报错");
        assert!(
            error.to_string().contains("无当前存在的 NVIDIA 显卡"),
            "{error}"
        );
    }

    #[test]
    fn key_path_round_trips_between_ps_and_rust_forms() {
        let instance = r"PCI\VEN_10DE&DEV_2491&SUBSYS_1462&REV_A1\4&2b3c4d5e&0&0008";
        let key = key_path_of(instance);
        assert_eq!(
            key,
            r"HKLM:\SYSTEM\CurrentControlSet\Enum\PCI\VEN_10DE&DEV_2491&SUBSYS_1462&REV_A1\4&2b3c4d5e&0&0008"
        );
        assert_eq!(instance_from_key_path(&key), instance);
        // PS 也认 HKEY_LOCAL_MACHINE 写法
        assert_eq!(
            instance_from_key_path(&format!(
                r"HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Enum\{instance}"
            )),
            instance
        );
    }

    #[test]
    fn state_capture_matches_the_ps_schema() {
        let item = device(
            r"PCI\VEN_10DE&DEV_2491&SUBSYS_1&4&1",
            "NVIDIA GeForce RTX 3080 Ti Laptop GPU",
        );
        let captured = GpuState::capture(
            &item,
            "NVIDIA GeForce RTX 5070",
            "2026-11-23T09:20:00Z".to_owned(),
        )
        .expect("备份");
        assert_eq!(
            captured.key_path,
            r"HKLM:\SYSTEM\CurrentControlSet\Enum\PCI\VEN_10DE&DEV_2491&SUBSYS_1&4&1"
        );
        assert_eq!(captured.original_value, item.device_desc);
        assert_eq!(
            captured.expected_value,
            r"@oem.inf,%dev%;NVIDIA GeForce RTX 5070"
        );
        assert_eq!(captured.target_name, "NVIDIA GeForce RTX 5070");
        // 序列化出来的键名必须是 PS 认得的
        let json = serde_json::to_string(&captured).expect("序列化");
        for key in [
            "KeyPath",
            "OriginalValue",
            "ExpectedValue",
            "TargetName",
            "CreatedAt",
        ] {
            assert!(json.contains(&format!("\"{key}\"")), "{key} 缺了：{json}");
        }
        // 读回来（PS 写出来的形状）
        let parsed: GpuState = serde_json::from_str(&json).expect("反序列化");
        assert_eq!(parsed, captured);
        assert!(parsed.belongs_to(&item));
        assert_eq!(
            parsed.original_display_name().expect("原名"),
            "NVIDIA GeForce RTX 3080 Ti Laptop GPU"
        );
    }

    #[test]
    fn state_reads_the_old_snake_case_shape_too() {
        let json = r#"{
            "instance_id": "PCI\\VEN_10DE&DEV_2491&SUBSYS_1&4&1",
            "original_value": "@oem.inf,%dev%;RTX 3080 Ti",
            "target_name": "RTX 5070"
        }"#;
        let parsed: GpuState = serde_json::from_str(json).expect("旧备份也要能读");
        assert_eq!(parsed.instance_id(), r"PCI\VEN_10DE&DEV_2491&SUBSYS_1&4&1");
        assert_eq!(parsed.original_display_name().expect("原名"), "RTX 3080 Ti");
        assert!(parsed.expected_value.is_empty(), "旧格式没有 ExpectedValue");
    }

    #[test]
    fn restore_refuses_when_the_value_changed_after_masking() {
        let item = device(r"PCI\VEN_10DE&DEV_2491&SUBSYS_1&4&1", "RTX 3080 Ti");
        let captured = state(&item.instance_id, "RTX 3080 Ti", "NVIDIA GeForce RTX 5070");
        assert_eq!(
            captured
                .restore_value(&captured.expected_value)
                .expect("值没变"),
            captured.original_value
        );
        let error = captured
            .restore_value(r"@oem.inf,%dev%;NVIDIA GeForce RTX 4090")
            .expect_err("被改过要拒绝");
        assert!(error.to_string().contains("被改过"), "{error}");
    }

    #[test]
    fn driver_desc_is_the_fallback_original_name() {
        assert_eq!(
            value_from_driver_desc(
                r"@oem.inf,%dev%;NVIDIA GeForce RTX 5070",
                "NVIDIA GeForce RTX 3080 Ti"
            )
            .expect("兜底"),
            r"@oem.inf,%dev%;NVIDIA GeForce RTX 3080 Ti"
        );
        assert!(value_from_driver_desc("x;y", "  ").is_err());
    }

    #[test]
    fn target_names_are_validated() {
        assert_eq!(
            validate_target_name("  NVIDIA GeForce RTX 5070  ").expect("正常"),
            "NVIDIA GeForce RTX 5070"
        );
        for (value, fragment) in [
            ("", "不能为空"),
            ("   ", "不能为空"),
            ("A;B", "分号"),
            ("A\"B", "双引号"),
            ("A\tB", "控制字符"),
        ] {
            let error = validate_target_name(value).expect_err(value);
            assert!(error.to_string().contains(fragment), "{value} → {error}");
        }
        let too_long = "名".repeat(MAX_TARGET_NAME_CHARS + 1);
        let error = validate_target_name(&too_long).expect_err("超长");
        assert!(error.to_string().contains("太长"), "{error}");
    }

    #[test]
    fn situation_decides_what_the_panel_can_do() {
        let instance = r"PCI\VEN_10DE&DEV_2491&SUBSYS_1&4&1";
        let real = device(instance, "NVIDIA GeForce RTX 3080 Ti");
        // 1) 干净状态：可以切换
        let clean = GpuSituation::build(real.clone(), Some("NVIDIA GeForce RTX 3080 Ti"), None, "");
        assert!(!clean.masked);
        assert!(clean.can_apply("NVIDIA GeForce RTX 5070"));
        assert!(!clean.can_apply("NVIDIA GeForce RTX 3080 Ti"), "同名不用切");
        assert!(!clean.can_restore());
        assert!(!clean.can_reset());

        // 2) 已伪装（有备份）：可以还原，不能再切、不能清备份
        let masked_device = device(instance, "NVIDIA GeForce RTX 5070");
        let backup = state(
            instance,
            "NVIDIA GeForce RTX 3080 Ti",
            "NVIDIA GeForce RTX 5070",
        );
        let masked = GpuSituation::build(
            masked_device,
            Some("NVIDIA GeForce RTX 3080 Ti"),
            Some(backup.clone()),
            "",
        );
        assert!(masked.masked);
        assert!(masked.can_restore());
        assert!(!masked.can_apply("NVIDIA GeForce RTX 4090"));
        assert!(!masked.can_reset(), "伪装态不能清备份（PS 同语义）");
        assert_eq!(
            masked.original_name.as_deref(),
            Some("NVIDIA GeForce RTX 3080 Ti")
        );

        // 3) 驱动更新后：名字回到真实值，但旧备份还在 → 只能清备份
        let stale = GpuSituation::build(
            real.clone(),
            Some("NVIDIA GeForce RTX 3080 Ti"),
            Some(backup),
            "",
        );
        assert!(!stale.masked);
        assert!(stale.can_reset());
        assert!(
            !stale.can_apply("NVIDIA GeForce RTX 5070"),
            "有旧备份要先清"
        );
        assert!(!stale.can_restore());

        // 4) 没有备份、读不到 DriverDesc：退回"当前名 == 目标名"判断，且不能还原
        let unknown = GpuSituation::build(
            device(instance, "NVIDIA GeForce RTX 5070"),
            None,
            None,
            "NVIDIA GeForce RTX 5070",
        );
        assert!(unknown.masked);
        assert!(!unknown.can_restore(), "不知道原名就不能还原");

        // 5) 备份属于别的设备（换过显卡）：不能切、不能还原，但必须能清除旧备份
        let foreign = GpuSituation::build(
            real.clone(),
            Some("NVIDIA GeForce RTX 3080 Ti"),
            Some(state(
                r"PCI\VEN_10DE&DEV_2482&SUBSYS_9&4&9",
                "NVIDIA GeForce RTX 3070",
                "NVIDIA GeForce RTX 4090",
            )),
            "",
        );
        assert!(foreign.backup_foreign);
        assert!(foreign.backup.is_some());
        assert!(foreign.owned_backup().is_none());
        assert!(
            !foreign.can_apply("NVIDIA GeForce RTX 5070"),
            "有旧备份先清"
        );
        assert!(foreign.can_reset(), "别的设备的旧备份也要能清");
        assert!(!foreign.can_restore(), "别的设备的备份不能拿来还原");
    }

    #[test]
    fn iso_timestamps_are_utc_with_a_z_suffix() {
        assert_eq!(format_iso8601_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_iso8601_utc(86_400), "1970-01-02T00:00:00Z");
        assert_eq!(format_iso8601_utc(1_700_000_000), "2023-11-14T22:13:20Z");
        // 真机 Kuro 埋点里的 1795414354 s（本地 2026-11-23 14:12:34 +08:00）
        assert_eq!(format_iso8601_utc(1_795_414_354), "2026-11-23T06:12:34Z");
    }
}
