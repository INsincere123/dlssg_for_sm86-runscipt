//! NVIDIA 显卡设备名的真实注册表读写（M5-01，IO 部分）。
//!
//! 高风险操作，因此把边界写死：
//!
//! - **读**：`HKLM\SYSTEM\CurrentControlSet\Enum\PCI\<VEN_10DE…>\<实例>` 的 `DeviceDesc`
//! - **写**：只允许改 `DeviceDesc` 这一个值，且必须先有状态备份；写入前要求进程已提权
//! - **备份**：`%ProgramData%\Toggle-NvidiaDeviceName\state.json`，与 PS 工具同路径
//! - 改完需要重启才生效，本模块不重启、不动驱动、不碰其他键

use std::path::PathBuf;

use launcher_core::gpu::{DeviceInfo, GpuState, STATE_DIRECTORY_NAME, STATE_FILE_NAME};
use launcher_core::{Error, Result};

use crate::registry::{self, HKEY_LOCAL_MACHINE};

const ENUM_PCI: &str = r"SYSTEM\CurrentControlSet\Enum\PCI";

/// 枚举当前存在的 NVIDIA 显卡（只读）。
pub fn enumerate_devices() -> Vec<DeviceInfo> {
    let mut devices = Vec::new();
    for vendor in registry::sub_keys(HKEY_LOCAL_MACHINE, ENUM_PCI) {
        if !vendor.to_ascii_uppercase().starts_with("VEN_10DE&") {
            continue;
        }
        let vendor_path = format!("{ENUM_PCI}\\{vendor}");
        for instance in registry::sub_keys(HKEY_LOCAL_MACHINE, &vendor_path) {
            let instance_path = format!("{vendor_path}\\{instance}");
            let Some(description) =
                registry::read_string(HKEY_LOCAL_MACHINE, &instance_path, "DeviceDesc")
            else {
                continue;
            };
            let friendly =
                registry::read_string(HKEY_LOCAL_MACHINE, &instance_path, "FriendlyName")
                    .unwrap_or_else(|| description.clone());
            let display_name = launcher_core::gpu::split_description(&description)
                .map(|item| item.display_name)
                .unwrap_or(friendly);
            devices.push(DeviceInfo {
                instance_id: format!(r"PCI\{vendor}\{instance}"),
                display_name,
                device_desc: description,
            });
        }
    }
    devices
}

/// 状态备份目录（`%ProgramData%\Toggle-NvidiaDeviceName`）。
pub fn state_directory() -> Option<PathBuf> {
    let program_data = std::env::var_os("ProgramData")?;
    Some(PathBuf::from(program_data).join(STATE_DIRECTORY_NAME))
}

/// 状态备份文件路径。
pub fn state_path() -> Option<PathBuf> {
    state_directory().map(|directory| directory.join(STATE_FILE_NAME))
}

/// 读回已保存的状态备份。
pub fn load_state() -> Option<GpuState> {
    let path = state_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(text.trim()).ok()
}

/// 写状态备份（原子写；已存在则不覆盖，除非显式要求）。
pub fn save_state(state: &GpuState, overwrite: bool) -> Result<PathBuf> {
    let directory = state_directory()
        .ok_or_else(|| Error::unavailable("环境变量 ProgramData 未设置，无法保存显卡状态备份。"))?;
    std::fs::create_dir_all(&directory).map_err(|error| Error::io(&directory, error))?;
    let path = directory.join(STATE_FILE_NAME);
    if path.exists() && !overwrite {
        return Err(Error::invalid(format!(
            "状态备份已存在，拒绝覆盖：{}",
            path.display()
        )));
    }
    let text = serde_json::to_string_pretty(state)
        .map_err(|error| Error::invalid(format!("状态备份无法序列化：{error}")))?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, text.as_bytes()).map_err(|error| Error::io(&temporary, error))?;
    std::fs::rename(&temporary, &path).map_err(|error| Error::io(&path, error))?;
    Ok(path)
}

/// 写入新的 `DeviceDesc`。**调用方必须先 `save_state`**，这里只做提权与键存在性检查。
pub fn write_device_desc(instance_id: &str, value: &str) -> Result<()> {
    if !crate::is_elevated() {
        return Err(Error::unavailable(
            "修改显卡设备名需要管理员权限；请用提权入口运行。",
        ));
    }
    if instance_id.len() < 10
        || !instance_id
            .to_ascii_uppercase()
            .starts_with("PCI\\VEN_10DE&")
    {
        return Err(Error::invalid(format!(
            "只允许修改 NVIDIA 显卡的 DeviceDesc：{instance_id}"
        )));
    }
    // DeviceDesc 里既有 `\` 也有实例串，注册表路径直接拼在 PCI 枚举下。
    let sub_key = format!("{ENUM_PCI}\\{}", instance_id.trim_start_matches("PCI\\"));
    registry::write_string(HKEY_LOCAL_MACHINE, &sub_key, "DeviceDesc", value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumerating_devices_is_read_only_and_parses_descriptions() {
        let devices = enumerate_devices();
        for device in &devices {
            assert!(
                device
                    .instance_id
                    .to_ascii_uppercase()
                    .starts_with("PCI\\VEN_10DE&"),
                "{}",
                device.instance_id
            );
            assert!(!device.device_desc.is_empty());
            // 每台的 DeviceDesc 都应该能被拆分（否则说明驱动写法超出预期，需要人工看）
            let description = launcher_core::gpu::split_description(&device.device_desc);
            assert!(
                description.is_ok(),
                "{}: {description:?}",
                device.instance_id
            );
        }
        eprintln!("枚举到 {} 台 NVIDIA 设备", devices.len());
    }

    #[test]
    fn state_path_is_under_program_data() {
        if std::env::var_os("ProgramData").is_none() {
            return;
        }
        let path = state_path().expect("路径");
        assert!(path.ends_with(STATE_FILE_NAME));
        assert!(path.to_string_lossy().contains(STATE_DIRECTORY_NAME));
    }

    #[test]
    fn write_refuses_non_nvidia_targets_before_touching_the_registry() {
        let error = write_device_desc(r"PCI\VEN_8086&DEV_46A6&SUBSYS_1", "x").expect_err("应拒绝");
        assert!(error.to_string().contains("只允许修改 NVIDIA"), "{error}");
    }
}
