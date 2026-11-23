//! NVIDIA 显卡设备名伪装（M5-01，纯逻辑部分）。
//!
//! 逐条对齐 `启动脚本/scripts/tools/Toggle-NvidiaDeviceName.ps1`：
//!
//! - `DeviceDesc` 形如 `@oemXX.inf,%nvidia_dev.xxxx%;NVIDIA GeForce RTX 3080 Ti Laptop GPU`：
//!   最后一个 `;` 之前是前缀（含分号），之后是显示名
//! - 没有 `;` 且非空、不以 `@` 开头 → 整个值就是显示名（前缀为空）
//! - 其余情况（以 `@` 开头却没有 `;`、或 `;` 在末尾）一律**拒绝覆盖**，避免把格式猜坏
//! - 设备筛选：`Class=Display`、实例 ID 以 `PCI\VEN_10DE&` 开头（大小写不敏感），
//!   必须唯一；零个或多个都报错并列出候选
//! - 状态备份写到 `%ProgramData%\Toggle-NvidiaDeviceName\state.json`

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

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

/// 一台显卡。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// `PCI\VEN_10DE&DEV_2491&SUBSYS_...\4&...`
    pub instance_id: String,
    /// 当前显示名（`DeviceDesc` 或 `FriendlyName` 里取到的）。
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
    let upper = instance_id.to_ascii_uppercase();
    upper.starts_with("PCI\\VEN_10DE&")
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
        "启动配置 Gpu.DeviceInstanceId 无法唯一确定 NVIDIA 显卡，请填写有效设备实例标识。\r\n{listing}"
    )))
}

/// 状态备份（`%ProgramData%\Toggle-NvidiaDeviceName\state.json`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuState {
    /// 伪装前的 `DeviceDesc` 原值。
    pub original_value: String,
    pub original_display_name: String,
    pub target_name: String,
    pub instance_id: String,
}

impl GpuState {
    /// 由当前设备与目标名构造备份。
    pub fn capture(device: &DeviceInfo, target: &str) -> Result<Self> {
        let description = split_description(&device.device_desc)?;
        Ok(Self {
            original_value: device.device_desc.clone(),
            original_display_name: description.display_name,
            target_name: target.to_owned(),
            instance_id: device.instance_id.clone(),
        })
    }

    /// 伪装后应写入的值。
    pub fn masked_value(&self) -> Result<String> {
        let description = split_description(&self.original_value)?;
        Ok(description.with_display_name(&self.target_name).compose())
    }
}

/// 状态备份目录名（`%ProgramData%\Toggle-NvidiaDeviceName`）。
pub const STATE_DIRECTORY_NAME: &str = "Toggle-NvidiaDeviceName";

/// 状态文件名。
pub const STATE_FILE_NAME: &str = "state.json";

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
    fn no_nvidia_device_reports_the_same_wording() {
        let devices = vec![device(r"PCI\VEN_8086&DEV_46A6&SUBSYS_1&4&2", "Intel UHD")];
        let error = select_device(&devices, None).expect_err("应报错");
        assert!(
            error.to_string().contains("无当前存在的 NVIDIA 显卡"),
            "{error}"
        );
    }

    #[test]
    fn state_captures_original_and_composes_the_masked_value() {
        let item = device(
            r"PCI\VEN_10DE&DEV_2491&SUBSYS_1&4&1",
            "NVIDIA GeForce RTX 3080 Ti Laptop GPU",
        );
        let state = GpuState::capture(&item, "NVIDIA GeForce RTX 4090 Laptop GPU").expect("备份");
        assert_eq!(
            state.original_display_name,
            "NVIDIA GeForce RTX 3080 Ti Laptop GPU"
        );
        assert_eq!(state.target_name, "NVIDIA GeForce RTX 4090 Laptop GPU");
        assert_eq!(
            state.masked_value().expect("写入值"),
            r"@oem.inf,%dev%;NVIDIA GeForce RTX 4090 Laptop GPU"
        );
        // 备份可 JSON 往返
        let json = serde_json::to_string(&state).expect("序列化");
        let parsed: GpuState = serde_json::from_str(&json).expect("反序列化");
        assert_eq!(parsed, state);
    }

    #[test]
    fn capture_refuses_a_malformed_device_desc() {
        let mut item = device(r"PCI\VEN_10DE&DEV_2491&SUBSYS_1&4&1", "x");
        item.device_desc = "@broken".to_owned();
        let error = GpuState::capture(&item, "target").expect_err("应拒绝");
        assert!(error.to_string().contains("unexpected format"), "{error}");
    }
}
