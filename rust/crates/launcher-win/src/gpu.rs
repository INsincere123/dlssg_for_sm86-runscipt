//! NVIDIA 显卡设备名的真实注册表读写（M5-01，IO 部分）。
//!
//! 高风险操作，因此把边界写死：
//!
//! - **读**：`HKLM\SYSTEM\CurrentControlSet\Enum\PCI\<VEN_10DE…>\<实例>` 的 `DeviceDesc`
//! - **读兜底**：设备的 `Driver` 值（`{类GUID}\0000`）→ `Control\Class\<该类\实例>` 的
//!   `DriverDesc`，用于"没有备份时还原原始名"（PS 的 `Get-FallbackOriginalDescription`）
//! - **写**：只允许改 `DeviceDesc` 这一个值，且必须先有状态备份；写入前要求进程已提权
//! - **备份**：`%ProgramData%\Toggle-NvidiaDeviceName\state.json`，与 PS 工具同路径、同字段
//! - 改完需要重启才生效，本模块不重启、不动驱动、不碰其他键

use std::path::PathBuf;

use launcher_core::gpu::{DeviceInfo, GpuState, STATE_DIRECTORY_NAME, STATE_FILE_NAME};
use launcher_core::{Error, Result};

use crate::registry::{self, HKEY_LOCAL_MACHINE};

const ENUM_PCI: &str = r"SYSTEM\CurrentControlSet\Enum\PCI";
const CLASS_ROOT: &str = r"SYSTEM\CurrentControlSet\Control\Class";
/// 显示适配器类（`Get-PnpDevice -Class Display` 就是它）。
const DISPLAY_CLASS_GUID: &str = "{4d36e968-e325-11ce-bfc1-08002be10318}";
/// `ConfigFlags` 的 `CONFIGFLAG_REMOVED`：设备已不在机器上（换槽/换驱动后的幽灵项）。
const CONFIGFLAG_REMOVED: u32 = 0x0000_0002;

/// 这台 VEN_10DE 设备是不是显示适配器。
///
/// 同一张 N 卡在 `Enum\PCI` 里还有 HD 音频功能（`VEN_10DE&DEV_2288` 之类），
/// 只按厂商过滤会把音频当成第二张显卡（真机 2026-11-23 就是这么误报的）。
fn is_display_adapter(instance_path: &str) -> bool {
    if let Some(guid) = registry::read_string(HKEY_LOCAL_MACHINE, instance_path, "ClassGUID") {
        return guid.trim().eq_ignore_ascii_case(DISPLAY_CLASS_GUID);
    }
    registry::read_string(HKEY_LOCAL_MACHINE, instance_path, "Class")
        .map(|class| class.trim().eq_ignore_ascii_case("Display"))
        .unwrap_or(false)
}

/// 设备是否已被移除（留着旧 key 但不在这台机器上）。
fn is_removed(instance_path: &str) -> bool {
    registry::read_dword(HKEY_LOCAL_MACHINE, instance_path, "ConfigFlags")
        .map(|flags| flags & CONFIGFLAG_REMOVED != 0)
        .unwrap_or(false)
}

/// 枚举当前存在的 NVIDIA 显卡（只读；只要显示适配器，不要音频等附带功能）。
pub fn enumerate_devices() -> Vec<DeviceInfo> {
    let mut devices = Vec::new();
    for vendor in registry::sub_keys(HKEY_LOCAL_MACHINE, ENUM_PCI) {
        if !vendor.to_ascii_uppercase().starts_with("VEN_10DE&") {
            continue;
        }
        let vendor_path = format!("{ENUM_PCI}\\{vendor}");
        for instance in registry::sub_keys(HKEY_LOCAL_MACHINE, &vendor_path) {
            let instance_path = format!("{vendor_path}\\{instance}");
            if !is_display_adapter(&instance_path) || is_removed(&instance_path) {
                continue;
            }
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

/// 设备对应的驱动类描述（`DriverDesc`）：没有状态备份时用它推断原始显示名。
///
/// 走的是 PS 的同一条路径：设备键的 `Driver` = `{类GUID}\0000`，
/// 再到 `Control\Class\{类GUID}\0000` 读 `DriverDesc`。
pub fn driver_desc(device: &DeviceInfo) -> Option<String> {
    let sub_key = format!(
        r"{ENUM_PCI}\{}",
        device.instance_id.trim_start_matches("PCI\\")
    );
    let driver = registry::read_string(HKEY_LOCAL_MACHINE, &sub_key, "Driver")?;
    let driver = driver.trim();
    if driver.is_empty() || driver.contains('/') {
        return None;
    }
    let class_key = format!(r"{CLASS_ROOT}\{}", driver.trim_start_matches('\\'));
    registry::read_string(HKEY_LOCAL_MACHINE, &class_key, "DriverDesc")
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

/// 读回已保存的状态备份；没有文件或格式不认识时返回 `None`（调用方按"没有备份"处理）。
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

/// 删除状态备份（PS 的 `-ResetState`）。文件不存在也算成功。
pub fn delete_state() -> Result<()> {
    let Some(path) = state_path() else {
        return Ok(());
    };
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::io(&path, error)),
    }
}

/// 只探测这台设备的 `DeviceDesc` 能不能以写权限打开（**不写任何值**）。
///
/// 用来区分两种失败：句柄没申请写权限（曾经的 bug）与键的 ACL 真不允许写。
pub fn writable(device: &DeviceInfo) -> bool {
    if !launcher_core::gpu::is_nvidia_instance(&device.instance_id) {
        return false;
    }
    let sub_key = format!(
        r"{ENUM_PCI}\{}",
        device.instance_id.trim_start_matches("PCI\\")
    );
    registry::can_write(HKEY_LOCAL_MACHINE, &sub_key)
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
            // 只要显示适配器：N 卡的 HD 音频功能也在 VEN_10DE 下，不能算第二张显卡
            assert!(
                !device
                    .device_desc
                    .to_ascii_lowercase()
                    .contains("audio controller"),
                "把音频功能当成显卡了：{} / {}",
                device.instance_id,
                device.device_desc
            );
            // 每台的 DeviceDesc 都应该能被拆分（否则说明驱动写法超出预期，需要人工看）
            let description = launcher_core::gpu::split_description(&device.device_desc);
            assert!(
                description.is_ok(),
                "{}: {description:?}",
                device.instance_id
            );
            // 驱动类描述要么读不到（老驱动），要么是能直接写回去的名字
            if let Some(driver) = driver_desc(device) {
                assert!(!driver.trim().is_empty(), "{}", device.instance_id);
            }
            eprintln!(
                "{} → {} / DriverDesc={:?}",
                device.instance_id,
                device.display_name,
                driver_desc(device)
            );
        }
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
