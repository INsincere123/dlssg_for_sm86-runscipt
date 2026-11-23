//! 设置页模型（M4-03）：界面与校验之间的唯一中间层，可无窗口测试。
//!
//! 12 个键分常用 / 高级两组，联动规则与 `settings::Settings::validate` 完全一致：
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

use launcher_core::settings::{EngineKind, Preset, RuntimeMode, Settings, SuperResolutionMode};

/// 哪些控件当前可用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldStates {
    pub advanced_enabled: bool,
    pub max_generated_frames: u8,
    pub super_resolution_tuning: bool,
    pub runtime_path: bool,
    pub preset_choice: bool,
}

/// 设置页状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsForm {
    pub settings: Settings,
    /// 最近一次校验失败的原因；`None` 表示无错。
    pub error: Option<String>,
    /// 是否有未保存的修改。
    pub dirty: bool,
}

impl SettingsForm {
    pub fn new(settings: Settings) -> Self {
        Self {
            settings,
            error: None,
            dirty: false,
        }
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
        match self.settings.validate() {
            Ok(()) => {
                self.error = None;
                true
            }
            Err(error) => {
                self.error = Some(error.to_string());
                false
            }
        }
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
    #[allow(dead_code)]
    pub fn is_verbatim(&self) -> bool {
        self.settings.custom_ini.is_some()
    }
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
}
