//! 设置页模型（M4-03）：界面与校验之间的唯一中间层，可无窗口测试。
//!
//! 从 2026-11-23 起拆成两个视图（用户要求）：
//!
//! - **帧生成**：组件的 12 个键（常用 5 + 高级 7）+ **代理文件**选择
//! - **画质**：UE 画质档位（`Tier1`..`Tier5`）+ `05-自定义` 的 INI 路径
//!
//! 12 个键的联动规则与 `settings::Settings::validate` 完全一致：
//!
//! | 键 | 组 | 联动 |
//! |---|---|---|
//! | `Enabled` | 常用 | 关闭时其余全部灰掉 |
//! | `Engine` | 常用 | 切到 XeSS 时 `MaxGeneratedFrames` 上限从 5 降到 3 并夹紧 |
//! | `MaxGeneratedFrames` | 常用 | 上限随引擎变化 |
//! | `SuperResolution.Mode` | 常用 | `Off` 时 `Level`/`Preset` 灰掉（不需要超分资源） |
//! | `Dlssg.Preset` | 常用 | 同上 |
//! | `Optimized` / `Level` | 高级 | `Level` 随超分模式灰化 |
//! | `Logging.Directory` | 高级 | 始终可编辑（非空、无非法字符） |
//! | `FlowResolution` / `UseHudless` | 高级 | 始终可编辑 |
//! | `Runtime.Mode` / `Runtime.Path` | 高级 | `Bundled` 时路径灰掉且必须为空 |
//! | 自定义 INI | 高级 | 有自定义内容时预设灰掉 |
//!
//! 代理与画质档位**不属于**这 12 个键：它们存在偏好里（`proxy_index` / `quality_tier`），
//! 由本模块一并搬运，因此界面只跟一个结构打交道。

use std::path::{Path, PathBuf};

use launcher_core::preferences::GameProfile;
use launcher_core::quality::{EngineHost, QualityTier};
use launcher_core::resources::PROXY_NAMES;
use launcher_core::settings::{EngineKind, Preset, RuntimeMode, Settings, SuperResolutionMode};

/// 两个视图。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsView {
    /// 帧生成参数、日志与代理文件。
    #[default]
    FrameGen,
    /// 画质档位与自定义 INI。
    Quality,
}

impl SettingsView {
    pub fn title(self) -> &'static str {
        match self {
            Self::FrameGen => "帧生成",
            Self::Quality => "画质",
        }
    }
}

/// 哪些控件当前可用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldStates {
    pub advanced_enabled: bool,
    pub max_generated_frames: u8,
    pub super_resolution_tuning: bool,
    pub runtime_path: bool,
    pub preset_choice: bool,
    /// 代理可以改（有未清理的部署时要先恢复）。
    pub proxy_choice: bool,
    /// 画质档位可以改（需要已确定 UE4/UE5）。
    pub quality_choice: bool,
    /// `05-自定义` 的路径输入框（只在 Tier5 时可用）。
    pub custom_ini_path: bool,
}

/// 一个代理选项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProxyOption {
    /// 1 起始编号（写日志与偏好都用它）。
    pub index: u8,
    pub name: &'static str,
    pub location: &'static str,
    pub note: &'static str,
}

/// 设置页状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsForm {
    /// 当前视图；保存时按视图决定要不要写画质字段。
    pub view: SettingsView,
    pub settings: Settings,
    /// 代理编号 1..6。
    pub proxy_index: u8,
    /// 画质档位。
    pub quality_tier: QualityTier,
    /// `05-自定义` 的 INI 路径（绝对或相对启动器根）。
    pub custom_ini_path: String,
    /// 引擎宿主：`none` / `UE4` / `UE5`（只展示 + 校准结果来自偏好）。
    pub engine_host: String,
    /// 是否已确定 UE4/UE5（画质档位的前提）。
    pub engine_host_is_ue: bool,
    /// 该游戏是否已有未清理的部署（有的话切换代理要先恢复）。
    pub deployment_pending: bool,
    /// 画质视图里显示的"当前配置源"。
    pub config_source: Option<String>,
    /// 最近一次校验失败的原因；`None` 表示无错。
    pub error: Option<String>,
    /// 是否有未保存的修改。
    pub dirty: bool,
}

impl SettingsForm {
    pub fn new(settings: Settings) -> Self {
        Self {
            view: SettingsView::FrameGen,
            settings,
            proxy_index: 1,
            quality_tier: QualityTier::default(),
            custom_ini_path: String::new(),
            engine_host: "none".to_owned(),
            engine_host_is_ue: false,
            deployment_pending: false,
            config_source: None,
            error: None,
            dirty: false,
        }
    }

    /// 按偏好里的一条记录填好表单（缺省值由 `GameProfile` 提供）。
    pub fn from_profile(view: SettingsView, profile: &GameProfile, pending: bool) -> Self {
        let mut form = Self::new(profile.settings.clone().unwrap_or_default());
        form.view = view;
        form.proxy_index = profile.proxy_index.clamp(1, PROXY_NAMES.len() as u8);
        form.quality_tier = QualityTier::from_text(&profile.quality_tier).unwrap_or_default();
        form.engine_host = profile.engine_host.clone();
        form.deployment_pending = pending;
        form.config_source = profile.config_source.clone();
        form.custom_ini_path = form
            .config_source
            .clone()
            .filter(|_| form.quality_tier == QualityTier::Tier5)
            .unwrap_or_default();
        form.engine_host_is_ue = EngineHost::from_text(&profile.engine_host)
            .map(|host| host != EngineHost::None)
            .unwrap_or(false);
        form
    }

    /// 控件联动状态。
    pub fn field_states(&self) -> FieldStates {
        let enabled = self.settings.enabled;
        let super_resolution_on =
            enabled && !matches!(self.settings.super_resolution, SuperResolutionMode::Off);
        FieldStates {
            advanced_enabled: enabled,
            max_generated_frames: match self.settings.engine {
                EngineKind::Dlss => 5,
                EngineKind::XeSs => 3,
            },
            super_resolution_tuning: super_resolution_on,
            runtime_path: enabled && self.settings.runtime_mode.needs_path(),
            preset_choice: enabled && self.settings.custom_ini.is_none(),
            proxy_choice: !self.deployment_pending,
            quality_choice: self.engine_host_is_ue,
            custom_ini_path: self.quality_tier == QualityTier::Tier5,
        }
    }

    /// 改引擎时夹紧受影响的字段（XeSS 最多 3 帧）。
    pub fn set_engine(&mut self, engine: EngineKind) {
        self.settings.engine = engine;
        let limit = match engine {
            EngineKind::Dlss => 5,
            EngineKind::XeSs => 3,
        };
        if self.settings.max_generated_frames > limit {
            self.settings.max_generated_frames = limit;
        }
        self.dirty = true;
    }

    /// 改超分模式：关闭时把档位留在默认值，避免残留无效组合。
    pub fn set_super_resolution(&mut self, mode: SuperResolutionMode) {
        self.settings.super_resolution = mode;
        if matches!(mode, SuperResolutionMode::Off) {
            self.settings.level = 1;
        }
        self.dirty = true;
    }

    /// 改运行时模式：Bundled 时清空路径（与 validate 的互斥规则一致）。
    pub fn set_runtime_mode(&mut self, mode: RuntimeMode) {
        self.settings.runtime_mode = mode;
        if !mode.needs_path() {
            self.settings.runtime_path = None;
        }
        self.dirty = true;
    }

    /// 设置自定义 INI；`None` 表示用预设（M4-04 的自定义 INI 页调用）。
    #[allow(dead_code)]
    pub fn set_custom_ini(&mut self, text: Option<String>) {
        self.settings.custom_ini = text.filter(|value| !value.trim().is_empty());
        self.dirty = true;
    }

    /// 校验并记录错误；返回是否通过。
    pub fn validate(&mut self) -> bool {
        if let Err(error) = self.settings.validate() {
            self.error = Some(error.to_string());
            return false;
        }
        if self.view == SettingsView::Quality {
            if let Err(message) = self.quality_source(Path::new(".")) {
                self.error = Some(message);
                return false;
            }
        }
        self.error = None;
        true
    }

    /// 保存前调用：只有校验通过才允许落盘。
    pub fn take_for_save(&mut self) -> Option<Settings> {
        if self.validate() {
            self.dirty = false;
            Some(self.settings.clone())
        } else {
            None
        }
    }
}

/// 一次导入的结果，供界面显示。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportOutcome {
    /// 成功识别的键。
    pub applied: Vec<String>,
    /// 不认识但保留在文件里的键。
    pub unknown: Vec<String>,
    /// 旧键名提示。
    pub hints: Vec<String>,
}

impl SettingsForm {
    /// 导入一份完整 INI：识别 12 键并填充表单，未知键原样报告（M4-04）。
    pub fn import_text(&mut self, text: &str) -> Result<ImportOutcome, String> {
        let mut probe = self.settings.clone();
        match probe.apply_ini(text) {
            Ok(report) => {
                self.settings = probe;
                self.dirty = true;
                Ok(ImportOutcome {
                    applied: report.applied,
                    unknown: report.unknown,
                    hints: report.hints,
                })
            }
            Err(error) => Err(format!("导入失败：{error}")),
        }
    }

    /// 把整份文本当作自定义 INI（原样使用，只由 `render` 补标记行）。
    pub fn import_verbatim(&mut self, text: &str) {
        let normalized = text.replace("\r\n", "\n").replace('\n', "\r\n");
        self.settings.custom_ini = Some(normalized);
        self.dirty = true;
    }

    /// 生成运行时配置文本（预览与部署同一份）。身份用固定占位值，避免预览与写入不一致。
    pub fn preview(&self) -> Result<String, String> {
        self.settings
            .render("preview")
            .map_err(|error| format!("预览失败：{error}"))
    }

    /// 恢复默认：清掉自定义 INI 与所有改动。
    pub fn restore_defaults(&mut self) {
        self.settings = Settings::default();
        self.error = None;
        self.dirty = true;
    }

    /// 是否处于"自定义全文"模式（界面与测试共用）。
    pub fn is_verbatim(&self) -> bool {
        self.settings.custom_ini.is_some()
    }
}

impl SettingsForm {
    /// 代理选项（编号、名称、来源、说明）。
    pub fn proxy_options() -> Vec<ProxyOption> {
        PROXY_NAMES
            .iter()
            .enumerate()
            .map(|(offset, name)| {
                let index = (offset + 1) as u8;
                ProxyOption {
                    index,
                    name,
                    location: if offset == 0 {
                        "资源根目录"
                    } else {
                        "alternatives"
                    },
                    note: ProxyOption::note_for(name),
                }
            })
            .collect()
    }

    /// 选中的代理名。
    pub fn proxy_name(&self) -> &'static str {
        PROXY_NAMES[(self.proxy_index.clamp(1, 6) - 1) as usize]
    }

    /// 改代理编号（超出 1..6 直接夹紧）。
    pub fn set_proxy(&mut self, index: u8) {
        let clamped = index.clamp(1, PROXY_NAMES.len() as u8);
        if clamped != self.proxy_index {
            self.proxy_index = clamped;
            self.dirty = true;
        }
    }

    /// 该游戏建议的代理（异环必须是 `winmm.dll`：ACE 会拦 `version.dll`）。
    pub fn recommended_proxy(game_id: &str, directory: &Path) -> u8 {
        let id = game_id.to_ascii_lowercase();
        let dir = directory.to_string_lossy().to_ascii_lowercase();
        if id == "nte" || id.contains("neverness") {
            return 2;
        }
        if dir.contains("neverness") || dir.contains("异环") {
            return 2;
        }
        1
    }

    /// 与建议不一致时的提示（不给建议时返回 `None`）。
    pub fn proxy_hint(&self, game_id: &str, directory: &Path) -> Option<String> {
        let recommended = Self::recommended_proxy(game_id, directory);
        if recommended == self.proxy_index {
            return None;
        }
        let reason = if recommended == 2 {
            "该游戏的反作弊会拦 version.dll，建议用 winmm.dll"
        } else {
            "默认用 version.dll"
        };
        Some(format!("{reason}（当前 {}）", self.proxy_name()))
    }

    /// 画质档位选项（4 档 + 05-自定义）。
    pub fn quality_options() -> [QualityTier; 5] {
        [
            QualityTier::Tier1,
            QualityTier::Tier2,
            QualityTier::Tier3,
            QualityTier::Tier4,
            QualityTier::Tier5,
        ]
    }

    pub fn set_quality(&mut self, tier: QualityTier) {
        if tier != self.quality_tier {
            self.quality_tier = tier;
            self.dirty = true;
        }
    }

    /// 画质配置源：`05-自定义` 用填的路径，其余按 UE 版本落到通用分类目录。
    pub fn quality_source(&self, root: &Path) -> Result<PathBuf, String> {
        let host = EngineHost::from_text(&self.engine_host)
            .map_err(|error| format!("引擎宿主无效：{error}"))?;
        if host == EngineHost::None {
            return Err(
                "尚未确定 UE4/UE5，暂时无法定位画质配置源；先在上面确认引擎版本。".to_owned(),
            );
        }
        if self.quality_tier == QualityTier::Tier5 {
            let text = self.custom_ini_path.trim();
            if text.is_empty() {
                return Err("05-自定义 需要填写一份 Engine.ini 路径。".to_owned());
            }
            let path = PathBuf::from(text);
            return Ok(if path.is_absolute() {
                path
            } else {
                root.join(path)
            });
        }
        launcher_core::quality::resolve(host, self.quality_tier, root)
            .map(|selection| selection.path)
            .map_err(|error| error.to_string())
    }

    /// 画质视图的一句话结论（界面直接贴，不用再算）。
    pub fn quality_summary(&self, root: &Path) -> String {
        match self.quality_source(root) {
            Ok(path) => {
                let exists = path.is_file();
                format!(
                    "配置源：{}{}",
                    path.display(),
                    if exists { "" } else { "（文件不存在）" }
                )
            }
            Err(message) => message,
        }
    }

    /// 保存时把与档位相关的字段写回偏好；只有画质视图才动配置源，避免误改个人配置。
    pub fn apply_to_profile(&self, profile: &mut GameProfile, root: &Path) -> Result<(), String> {
        profile.proxy_index = self.proxy_index;
        profile.quality_tier = self.quality_tier.as_text().to_owned();
        if self.view != SettingsView::Quality {
            return Ok(());
        }
        let path = self.quality_source(root)?;
        if !path.is_file() {
            return Err(format!(
                "配置源不存在：{}。换一个档位，或先把这个 Engine.ini 放到该位置。",
                path.display()
            ));
        }
        profile.config_source = Some(path.display().to_string());
        Ok(())
    }
}

impl ProxyOption {
    /// 每个代理的用途说明（口径与 `启动脚本/README.md` 的代理表一致）。
    fn note_for(name: &str) -> &'static str {
        match name {
            "version.dll" => "最通用，多数 UE 游戏可用；反作弊可能拦",
            "winmm.dll" => "异环必须用它（ACE 不拦）",
            "dbghelp.dll" => "少数游戏的备选",
            "dinput8.dll" => "少数游戏的备选",
            "dxgi.dll" => "游戏导入 dxgi 时更稳",
            "d3d12.dll" => "游戏导入 d3d12 时更稳",
            _ => "",
        }
    }
}

/// 12 个键的悬停说明。
///
/// 口径 B（用户确认）：**先读仓库文档与组件 INI，再对着代码行为改写成界面用语**，
/// 不凭印象写。来源是仓库根目录 `dlssg_sm86.ini` 的逐键注释（组件自带、双语），
/// 数字与默认值以它为准；界面只做压缩，不加文档里没有的承诺。
pub const KEY_HINTS: [(&str, &str); 12] = [
    (
        "Enabled",
        "1 = 用随包的 Ampere 优化运行库开帧生成（默认）。\n\
         0 = 关：游戏自带的 DLSS-G 原样加载，Ampere 上等于没有帧生成。\n\
         两种情况下代理都仍然转发真正的系统 DLL。",
    ),
    (
        "Engine",
        "DLSS = NVIDIA DLSS-G 模型跑本项目的 SM86/SM75 内核（默认）。\n\
         XeSS = Intel XeSS-FG model 15 跑本项目自己的运行器，不用 Intel 的 libxess_fg.dll。\n\
         XeSS 跑不了的后缓冲格式（RGBA16F）直接拒绝，不会退回 DLSS。",
    ),
    (
        "MaxGeneratedFrames",
        "每个真实帧生成几帧：3 = 4X（出厂），5 = 6X（随包运行库 310.9.16 的上限）。\n\
         这是「要求」不是上限，会原样告诉游戏，游戏菜单可以选更低。\n\
         做不到时不启用帧生成并记日志：超过 5、Engine=XeSS 时超过 3、只支持 2X 的旧插件。",
    ),
    (
        "SuperResolution.Mode",
        "DLSS 超分，仅 RTX 30/20。Off = 不介入（默认）。\n\
         M/L-INT8 = 预设 M/L 跑本项目的 INT8 内核；M/L-FP16 = 同一预设跑 NVIDIA 自己的内核。\n\
         画质 / 平衡 / DLAA 档用 M 或 L 比游戏自己的预设 K 慢。",
    ),
    (
        "Dlssg.Preset",
        "A（默认）= 不做 UI 重组、不开轻量插帧。\n\
         B = UI 重组：只有游戏同时交出无 HUD 画面和 UI 平面才有效果，多数游戏不交，那时与 A 相同。\n\
         C = 轻量插帧：每个生成帧的 GPU 时间少 32-54%，画质略低（主要在 HUD 与遮挡边缘）。",
    ),
    (
        "Optimized",
        "一致性档位，数字越大越快、离官方画面越远。\n\
         0 = 原厂内核；1 = 全部逐位一致的加速（默认）；\n\
         2 = 再加有损图像内核（真实 4K 实拍对官方输出 PSNR 44-53 dB，并排看不出差别）；\n\
         3 = 全部有损加速，最快。2 / 3 不再逐位一致。",
    ),
    (
        "Level",
        "0 = 关闭；1 = 错误（默认）；2 = 配置与能力；3 = 内核与 Evaluate 追踪。\n\
         所有模块、所有进程都追加写 Directory\\dlssg_sm86.log 这一个文件，\n\
         每行先是本地时间、pid、EXE 名和级别字母，再看 JSON 记录。",
    ),
    (
        "Logging.Directory",
        "日志目录，默认 dlssg_sm86\\logs（相对代理所在目录）。\n\
         一个游戏的所有模块共写这个目录下的 dlssg_sm86.log，\n\
         只看一个游戏时按 EXE 名或 pid 过滤。",
    ),
    (
        "FlowResolution",
        "只对 Engine=XeSS 生效。光流分辨率 Coarse / Middle / Fine，越细越慢（默认 Middle）。\n\
         CoarseAt4K = 输出像素 ≥ 3840×2160 时用 Coarse，否则 Middle。",
    ),
    (
        "UseHudless",
        "只对 Engine=XeSS 生效。\n\
         Auto = 与 0 相同（默认）：后缓冲作为网络输入，整幅画面含 UI 一起插值。\n\
         1 = 一律用游戏的无 HUD 画面做网络输入（没给或尺寸不同就报错退出）；0 = 不做 UI 合成。",
    ),
    (
        "Runtime.Mode",
        "Bundled = 用随包的运行库 dlssg_sm86\\runtime\\nvngx_dlssg.dll（正常用这个）。\n\
         External / Pinned 是进阶用法（见组件 docs\\INSTALL.md）。\n\
         运行库用不了时帧生成不启用、日志记一条错误，不会改用别的运行库顶上。",
    ),
    (
        "Runtime.Path",
        "External / Pinned 时指定的运行库文件路径；Bundled 时必须留空。\n\
         路径指错文件同样只是让帧生成不启用，并记一条错误。",
    ),
];

/// 取某个键的悬停说明；未知键返回空串（界面直接贴，不 panic）。
pub fn key_hint(key: &str) -> &'static str {
    KEY_HINTS
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, hint)| *hint)
        .unwrap_or("")
}

/// 常用组的 5 个键名（顺序即界面顺序）。
pub const COMMON_KEYS: [&str; 5] = [
    "Enabled",
    "Engine",
    "MaxGeneratedFrames",
    "SuperResolution.Mode",
    "Dlssg.Preset",
];

/// 高级组的 7 个键名（界面顺序）。
pub const ADVANCED_KEYS: [&str; 7] = [
    "Optimized",
    "Level",
    "Logging.Directory",
    "FlowResolution",
    "UseHudless",
    "Runtime.Mode",
    "Runtime.Path",
];

/// 全部 12 个键（含自定义 INI 之外的持久化键）。
#[allow(dead_code)]
pub fn all_keys() -> Vec<&'static str> {
    COMMON_KEYS
        .iter()
        .chain(ADVANCED_KEYS.iter())
        .copied()
        .collect()
}

/// 预设可选值（界面下拉用）。
#[allow(dead_code)]
pub fn preset_options() -> Vec<Preset> {
    // 组件只认 A/B/C 三档（与 settings::Preset 一致）。
    vec![Preset::A, Preset::B, Preset::C]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form() -> SettingsForm {
        SettingsForm::new(Settings::default())
    }

    #[test]
    fn twelve_keys_are_split_five_plus_seven() {
        assert_eq!(COMMON_KEYS.len(), 5);
        assert_eq!(ADVANCED_KEYS.len(), 7);
        assert_eq!(all_keys().len(), 12);
        assert_eq!(all_keys()[0], "Enabled");
        assert_eq!(all_keys()[11], "Runtime.Path");
        assert_eq!(preset_options().len(), 3);
    }

    #[test]
    fn defaults_pass_validation_and_are_not_dirty() {
        let mut form = form();
        assert!(form.validate(), "{:?}", form.error);
        assert!(!form.dirty);
        assert!(form.take_for_save().is_some());
    }

    #[test]
    fn switching_to_xess_clamps_frames_to_three() {
        let mut form = form();
        form.settings.max_generated_frames = 5;
        form.set_engine(EngineKind::XeSs);
        assert_eq!(form.settings.max_generated_frames, 3);
        assert_eq!(form.field_states().max_generated_frames, 3);
        assert!(form.validate(), "夹紧后应通过校验");
        // 回到 DLSS 不会自动放大，但允许用户再调高
        form.set_engine(EngineKind::Dlss);
        assert_eq!(form.settings.max_generated_frames, 3);
        assert_eq!(form.field_states().max_generated_frames, 5);
    }

    #[test]
    fn invalid_frames_fail_validation_with_a_message() {
        let mut form = form();
        form.settings.max_generated_frames = 6;
        assert!(!form.validate());
        let message = form.error.clone().expect("应有错误");
        assert!(message.contains("MaxGeneratedFrames"), "{message}");
        assert!(form.take_for_save().is_none(), "校验失败不能保存");
    }

    #[test]
    fn super_resolution_off_disables_tuning_and_resets_level() {
        let mut form = form();
        form.settings.level = 3;
        form.set_super_resolution(SuperResolutionMode::Off);
        assert_eq!(form.settings.level, 1);
        assert!(!form.field_states().super_resolution_tuning);
        form.set_super_resolution(SuperResolutionMode::MInt8);
        assert!(form.field_states().super_resolution_tuning);
    }

    #[test]
    fn runtime_mode_controls_the_path_field() {
        let mut form = form();
        // 默认 Bundled：路径灰掉且不能有值
        assert!(!form.field_states().runtime_path);
        form.settings.runtime_path = Some(r"D:\rt".to_owned());
        assert!(!form.validate(), "Bundled 带路径应失败");
        form.set_runtime_mode(RuntimeMode::Bundled);
        assert!(form.settings.runtime_path.is_none());
        assert!(form.validate());

        form.set_runtime_mode(RuntimeMode::External);
        assert!(form.field_states().runtime_path);
        assert!(!form.validate(), "需要路径的模式缺路径应失败");
        form.settings.runtime_path = Some(r"D:\rt".to_owned());
        assert!(form.validate());
    }

    #[test]
    fn import_applies_known_keys_and_reports_unknown_ones() {
        let mut form = form();
        let text = "\
[General]\r
Enabled=0\r
[FrameGeneration]\r
Engine=XeSS\r
MaxGeneratedFrames=2\r
Mystery=42\r
";
        let outcome = form.import_text(text).expect("导入");
        assert!(!form.settings.enabled);
        assert_eq!(form.settings.engine, EngineKind::XeSs);
        assert_eq!(form.settings.max_generated_frames, 2);
        assert!(
            outcome.unknown.iter().any(|key| key.contains("Mystery")),
            "{:?}",
            outcome.unknown
        );
        assert!(form.dirty);
    }

    #[test]
    fn broken_import_leaves_the_form_untouched() {
        let mut form = form();
        let before = form.settings.clone();
        let error = form
            .import_text("[General]\nEnabled=maybe\n")
            .expect_err("应报错");
        assert!(error.contains("导入失败"), "{error}");
        assert_eq!(form.settings, before, "失败不能改表单");
    }

    #[test]
    fn verbatim_import_normalizes_to_crlf_and_marks_preview() {
        let mut form = form();
        form.import_verbatim("[General]\nEnabled=0\n");
        assert!(form.is_verbatim());
        let preview = form.preview().expect("预览");
        assert!(
            preview.starts_with("; dlssg-launcher-config=preview"),
            "{preview}"
        );
        assert!(preview.contains("[General]\r\nEnabled=0\r\n"), "{preview}");
        assert!(!form.field_states().preset_choice, "自定义全文时预设应灰掉");
    }

    #[test]
    fn restore_defaults_clears_custom_ini_and_fields() {
        let mut form = form();
        form.import_verbatim("[General]\nEnabled=0\n");
        form.settings.engine = EngineKind::XeSs;
        form.restore_defaults();
        assert!(!form.is_verbatim());
        assert_eq!(form.settings, Settings::default());
        assert!(form.dirty);
    }

    #[test]
    fn preview_of_defaults_contains_all_sections() {
        let form = form();
        let preview = form.preview().expect("预览");
        for section in [
            "[General]",
            "[FrameGeneration]",
            "[DLSSG]",
            "[SuperResolution]",
            "[Logging]",
            "[XeSSFG]",
            "[Runtime]",
        ] {
            assert!(preview.contains(section), "缺少 {section}");
        }
        assert!(preview.ends_with("; dlssg-launcher-config=preview\r\n"));
    }

    #[test]
    fn custom_ini_disables_preset_choice() {
        let mut form = form();
        assert!(form.field_states().preset_choice);
        form.set_custom_ini(Some("; custom\n".to_owned()));
        assert!(form.settings.custom_ini.is_some());
        assert!(!form.field_states().preset_choice);
        form.set_custom_ini(Some("   ".to_owned()));
        assert!(form.settings.custom_ini.is_none(), "空白内容按清除处理");
        assert!(form.field_states().preset_choice);
    }

    #[test]
    fn every_key_has_a_non_empty_hint() {
        assert_eq!(KEY_HINTS.len(), 12);
        for key in all_keys() {
            let hint = key_hint(key);
            assert!(!hint.is_empty(), "{key} 没有悬停说明");
            assert!(hint.len() > 20, "{key} 的说明太短：{hint}");
        }
        // 名单与 12 键完全一致（顺序也一致），避免漏键或多键
        let names: Vec<&str> = KEY_HINTS.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, all_keys());
        assert_eq!(key_hint("NotAKey"), "");
    }

    #[test]
    fn proxy_options_cover_all_six_names_in_order() {
        let options = SettingsForm::proxy_options();
        assert_eq!(options.len(), 6);
        assert_eq!(options[0].index, 1);
        assert_eq!(options[0].name, "version.dll");
        assert_eq!(options[0].location, "资源根目录");
        assert_eq!(options[1].name, "winmm.dll");
        assert_eq!(options[1].location, "alternatives");
        assert_eq!(options[5].name, "d3d12.dll");
        assert!(options.iter().all(|option| !option.note.is_empty()));
    }

    #[test]
    fn proxy_selection_clamps_and_marks_dirty() {
        let mut form = form();
        assert_eq!(form.proxy_name(), "version.dll");
        form.set_proxy(2);
        assert_eq!(form.proxy_name(), "winmm.dll");
        assert!(form.dirty);
        form.set_proxy(99);
        assert_eq!(form.proxy_index, 6);
        form.set_proxy(0);
        assert_eq!(form.proxy_index, 1);
    }

    #[test]
    fn nte_recommends_winmm_and_others_version() {
        assert_eq!(SettingsForm::recommended_proxy("nte", Path::new("")), 2);
        assert_eq!(
            SettingsForm::recommended_proxy(
                "direct-1",
                Path::new(r"C:\Games\Neverness To Everness")
            ),
            2,
            "目录名也能认出来"
        );
        assert_eq!(
            SettingsForm::recommended_proxy(
                "direct-1",
                Path::new(r"C:\Games\Wuthering Waves Game")
            ),
            1
        );

        let mut form = form();
        let hint = form
            .proxy_hint("nte", Path::new(""))
            .expect("用 version 时应提示");
        assert!(hint.contains("winmm.dll"), "{hint}");
        form.set_proxy(2);
        assert!(form.proxy_hint("nte", Path::new("")).is_none());
    }

    #[test]
    fn proxy_choice_locks_while_a_deployment_is_pending() {
        let mut form = form();
        assert!(form.field_states().proxy_choice);
        form.deployment_pending = true;
        assert!(!form.field_states().proxy_choice, "先恢复才能换代理");
    }

    #[test]
    fn quality_source_follows_the_ue_major_and_tier_folder() {
        let mut form = form();
        form.view = SettingsView::Quality;
        form.engine_host = "UE5".to_owned();
        form.set_quality(QualityTier::Tier3);
        let root = Path::new(r"D:\Games\dlssg_for_sm86");
        assert_eq!(
            form.quality_source(root).expect("解析"),
            PathBuf::from(r"D:\Games\dlssg_for_sm86\配置\虚幻通用分类\UE5\03-性能优先\Engine.ini")
        );
        assert!(form.quality_summary(root).contains("配置源"));

        form.engine_host = "none".to_owned();
        let error = form.quality_source(root).expect_err("未确定引擎应报错");
        assert!(error.contains("尚未确定 UE4/UE5"), "{error}");
        assert!(!form.field_states().quality_choice);
    }

    #[test]
    fn custom_tier_needs_a_path_and_accepts_relative_ones() {
        let mut form = form();
        form.view = SettingsView::Quality;
        form.engine_host = "UE4".to_owned();
        form.set_quality(QualityTier::Tier5);
        assert!(form.field_states().custom_ini_path);
        let error = form
            .quality_source(Path::new(r"D:\root"))
            .expect_err("空路径应报错");
        assert!(error.contains("05-自定义"), "{error}");

        form.custom_ini_path = r"配置\我的\Engine.ini".to_owned();
        assert_eq!(
            form.quality_source(Path::new(r"D:\root")).expect("解析"),
            PathBuf::from(r"D:\root\配置\我的\Engine.ini")
        );
        form.custom_ini_path = r"E:\abs\Engine.ini".to_owned();
        assert_eq!(
            form.quality_source(Path::new(r"D:\root"))
                .expect("绝对路径"),
            PathBuf::from(r"E:\abs\Engine.ini")
        );
    }

    #[test]
    fn saving_the_frame_gen_view_leaves_the_config_source_alone() {
        let mut profile = GameProfile::new(
            "wuwa",
            "鸣潮",
            PathBuf::from(r"C:\Games\Wuthering Waves Game"),
        );
        profile.config_source = Some(r"D:\root\配置\鸣潮\Engine.ini".to_owned());
        let mut form = form();
        form.view = SettingsView::FrameGen;
        form.set_proxy(3);
        form.apply_to_profile(&mut profile, Path::new(r"D:\root"))
            .expect("保存");
        assert_eq!(profile.proxy_index, 3);
        assert_eq!(
            profile.config_source.as_deref(),
            Some(r"D:\root\配置\鸣潮\Engine.ini"),
            "帧生成视图不能改画质配置源"
        );
    }

    #[test]
    fn saving_the_quality_view_rewrites_the_config_source() {
        // 只有配置源真的存在才允许写进偏好：先造一棵最小目录树。
        let root = std::env::temp_dir().join(format!("dlssg-quality-{}-save", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let tier = root
            .join("配置")
            .join("虚幻通用分类")
            .join("UE5")
            .join("01-高性能均衡");
        std::fs::create_dir_all(&tier).expect("临时目录");
        std::fs::write(tier.join("Engine.ini"), b"[SystemSettings]\n").expect("写");

        let mut profile = GameProfile::new("pbz", "PBZ", PathBuf::from(r"C:\PBZ"));
        profile.engine_host = "UE5".to_owned();
        let mut form = SettingsForm::from_profile(SettingsView::Quality, &profile, false);
        form.set_quality(QualityTier::Tier1);
        form.apply_to_profile(&mut profile, &root).expect("保存");
        assert_eq!(profile.quality_tier, "Tier1");
        let expected = tier.join("Engine.ini").to_string_lossy().into_owned();
        assert_eq!(profile.config_source.as_deref(), Some(expected.as_str()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_quality_source_is_refused_instead_of_written() {
        let root =
            std::env::temp_dir().join(format!("dlssg-quality-{}-missing", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("临时目录");
        let mut profile = GameProfile::new("pbz", "PBZ", PathBuf::from(r"C:\PBZ"));
        profile.engine_host = "UE5".to_owned();
        profile.config_source = Some(r"D:\keep\Engine.ini".to_owned());
        let mut form = SettingsForm::from_profile(SettingsView::Quality, &profile, false);
        form.set_quality(QualityTier::Tier2);
        let error = form
            .apply_to_profile(&mut profile, &root)
            .expect_err("缺文件应拒绝");
        assert!(error.contains("配置源不存在"), "{error}");
        assert_eq!(
            profile.config_source.as_deref(),
            Some(r"D:\keep\Engine.ini"),
            "拒绝保存时不能改原有配置源"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn from_profile_reads_proxy_tier_and_pending_state() {
        let mut profile = GameProfile::new("nte", "异环", PathBuf::from(r"C:\Games\NTE"));
        profile.proxy_index = 2;
        profile.quality_tier = "Tier4".to_owned();
        profile.engine_host = "UE4".to_owned();
        profile.config_source = Some(r"D:\root\配置\异环\Engine.ini".to_owned());
        let form = SettingsForm::from_profile(SettingsView::Quality, &profile, true);
        assert_eq!(form.view, SettingsView::Quality);
        assert_eq!(form.proxy_index, 2);
        assert_eq!(form.quality_tier, QualityTier::Tier4);
        assert!(!form.field_states().proxy_choice, "有遗留部署");
        assert!(form.field_states().quality_choice);
        // 非 Tier5 时不把现有配置源当自定义路径
        assert!(form.custom_ini_path.is_empty());
    }
}
