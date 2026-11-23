//! 运行时配置模型与诊断模板（Phase 1 步 12–13）。
//!
//! 两部分：
//!
//! 1. **12 键模型**（步 12）：结构化设置 → 生成完整的 `dlssg_sm86` 运行配置文本。
//!    与现有 PowerShell 的关系见 PARITY D-2：模板只有 8 键、默认 `5/2/2`，
//!    这里按 0.4.2 发行配置对齐到 **12 键、`3/1/1`**，属于记录在案的有意偏离。
//! 2. **诊断模板**（步 13）：`Set-LauncherDiagnosticOptions` 与
//!    `Test-LauncherDiagnosticOptionsText` 的等价实现，用于生成临时诊断 INI
//!    与"无记录回收"时的精确文本识别。

use crate::config;
use crate::error::{Error, Result};

/// 身份标记行的前缀。恢复流程靠它识别"这份文件是我们生成的"。
pub const MARKER_PREFIX: &str = "; dlssg-launcher-config=";

// ---------------------------------------------------------------- 枚举

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineKind {
    Dlss,
    XeSs,
}

impl EngineKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EngineKind::Dlss => "DLSS",
            EngineKind::XeSs => "XeSS",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value.trim() {
            "DLSS" => Ok(EngineKind::Dlss),
            "XeSS" => Ok(EngineKind::XeSs),
            other => Err(Error::invalid(format!("Engine 取值非法：{other}"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    A,
    B,
    C,
}

impl Preset {
    pub fn as_str(self) -> &'static str {
        match self {
            Preset::A => "A",
            Preset::B => "B",
            Preset::C => "C",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value.trim() {
            "A" => Ok(Preset::A),
            "B" => Ok(Preset::B),
            "C" => Ok(Preset::C),
            other => Err(Error::invalid(format!(
                "Preset 取值非法：{other}（Auto 已于 2026-10-22 删除，按 A 运行）"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuperResolutionMode {
    Off,
    MInt8,
    LInt8,
    MFp16,
    LFp16,
}

impl SuperResolutionMode {
    pub const ALL: [SuperResolutionMode; 5] = [
        SuperResolutionMode::Off,
        SuperResolutionMode::MInt8,
        SuperResolutionMode::LInt8,
        SuperResolutionMode::MFp16,
        SuperResolutionMode::LFp16,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            SuperResolutionMode::Off => "Off",
            SuperResolutionMode::MInt8 => "M-INT8",
            SuperResolutionMode::LInt8 => "L-INT8",
            SuperResolutionMode::MFp16 => "M-FP16",
            SuperResolutionMode::LFp16 => "L-FP16",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value.trim() {
            "Off" => Ok(SuperResolutionMode::Off),
            "M-INT8" => Ok(SuperResolutionMode::MInt8),
            "L-INT8" => Ok(SuperResolutionMode::LInt8),
            "M-FP16" => Ok(SuperResolutionMode::MFp16),
            "L-FP16" => Ok(SuperResolutionMode::LFp16),
            other => Err(Error::invalid(format!(
                "SuperResolution.Mode 取值非法：{other}"
            ))),
        }
    }

    pub fn is_off(self) -> bool {
        self == SuperResolutionMode::Off
    }

    /// 超分是否启用 → 决定 `dlssg_sm86\sr` 资源是否必需（PARITY N-5）。
    pub fn requires_super_resolution_resources(self) -> bool {
        !self.is_off()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowResolution {
    Coarse,
    Middle,
    Fine,
    CoarseAt4K,
}

impl FlowResolution {
    pub fn as_str(self) -> &'static str {
        match self {
            FlowResolution::Coarse => "Coarse",
            FlowResolution::Middle => "Middle",
            FlowResolution::Fine => "Fine",
            FlowResolution::CoarseAt4K => "CoarseAt4K",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value.trim() {
            "Coarse" => Ok(FlowResolution::Coarse),
            "Middle" => Ok(FlowResolution::Middle),
            "Fine" => Ok(FlowResolution::Fine),
            "CoarseAt4K" => Ok(FlowResolution::CoarseAt4K),
            other => Err(Error::invalid(format!(
                "XeSSFG.FlowResolution 取值非法：{other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UseHudless {
    Auto,
    Zero,
    One,
}

impl UseHudless {
    pub fn as_str(self) -> &'static str {
        match self {
            UseHudless::Auto => "Auto",
            UseHudless::Zero => "0",
            UseHudless::One => "1",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value.trim() {
            "Auto" => Ok(UseHudless::Auto),
            "0" => Ok(UseHudless::Zero),
            "1" => Ok(UseHudless::One),
            other => Err(Error::invalid(format!(
                "XeSSFG.UseHudless 取值非法：{other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMode {
    Bundled,
    External,
    Pinned,
}

impl RuntimeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            RuntimeMode::Bundled => "Bundled",
            RuntimeMode::External => "External",
            RuntimeMode::Pinned => "Pinned",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value.trim() {
            "Bundled" => Ok(RuntimeMode::Bundled),
            "External" => Ok(RuntimeMode::External),
            "Pinned" => Ok(RuntimeMode::Pinned),
            other => Err(Error::invalid(format!(
                "Runtime.Mode 取值非法：{other}（Auto 已在 0.4.0 删除，按 Bundled 运行）"
            ))),
        }
    }

    pub fn needs_path(self) -> bool {
        self != RuntimeMode::Bundled
    }
}

// ---------------------------------------------------------------- 模型

/// 12 键设置。字段名与 `dlssg_sm86` 的键一一对应。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    pub engine: EngineKind,
    pub max_generated_frames: u8,
    pub preset: Preset,
    /// 一致性档位 0–3，越大越快、离官方画面越远。
    pub optimized: u8,
    pub super_resolution: SuperResolutionMode,
    pub level: u8,
    pub directory: String,
    pub flow_resolution: FlowResolution,
    pub use_hudless: UseHudless,
    pub runtime_mode: RuntimeMode,
    /// 仅在 `External`/`Pinned` 时有意义。
    pub runtime_path: Option<String>,
    /// 完整自定义 INI：有值时按原样写回（只加标记行），不生成结构化文本。
    pub custom_ini: Option<String>,
}

impl Default for Settings {
    /// 出厂值：与 0.4.2 发行配置一致（`3 / A / 1 / Off / 1`）。
    fn default() -> Self {
        Self {
            enabled: true,
            engine: EngineKind::Dlss,
            max_generated_frames: 3,
            preset: Preset::A,
            optimized: 1,
            super_resolution: SuperResolutionMode::Off,
            level: 1,
            directory: "dlssg_sm86\\logs".to_owned(),
            flow_resolution: FlowResolution::Middle,
            use_hudless: UseHudless::Auto,
            runtime_mode: RuntimeMode::Bundled,
            runtime_path: None,
            custom_ini: None,
        }
    }
}

impl Settings {
    /// 逐项校验。界面在写入前调用；范围与联动规则都在这里。
    pub fn validate(&self) -> Result<()> {
        let max = match self.engine {
            EngineKind::Dlss => 5,
            EngineKind::XeSs => 3,
        };
        if self.max_generated_frames < 1 || self.max_generated_frames > max {
            return Err(Error::invalid(format!(
                "MaxGeneratedFrames 必须在 1–{max} 之间（Engine={}）：{}",
                self.engine.as_str(),
                self.max_generated_frames
            )));
        }
        if self.optimized > 3 {
            return Err(Error::invalid(format!(
                "Optimized 必须在 0–3 之间：{}",
                self.optimized
            )));
        }
        if self.level > 3 {
            return Err(Error::invalid(format!(
                "Level 必须在 0–3 之间：{}",
                self.level
            )));
        }
        if self.directory.trim().is_empty() {
            return Err(Error::invalid("Logging.Directory 不能为空。"));
        }
        if self.directory.contains(['\r', '\n', '"']) {
            return Err(Error::invalid(format!(
                "Logging.Directory 含非法字符：{}",
                self.directory
            )));
        }
        match (&self.runtime_mode, &self.runtime_path) {
            (RuntimeMode::Bundled, Some(path)) if !path.trim().is_empty() => Err(Error::invalid(
                "Runtime.Mode=Bundled 时不能设置 Runtime.Path（界面应禁用该项）。",
            )),
            (mode, None) if mode.needs_path() => Err(Error::invalid(format!(
                "Runtime.Mode={} 必须提供 Runtime.Path。",
                mode.as_str()
            ))),
            (mode, Some(path)) if mode.needs_path() && path.trim().is_empty() => {
                Err(Error::invalid(format!(
                    "Runtime.Mode={} 的 Runtime.Path 不能为空。",
                    mode.as_str()
                )))
            }
            _ => Ok(()),
        }
    }

    /// 超分是否启用（资源断言用）。
    pub fn requires_super_resolution_resources(&self) -> bool {
        self.super_resolution.requires_super_resolution_resources()
    }

    /// 生成运行时配置文本：CRLF、UTF-8（无 BOM）、同样设置逐字节相同。
    ///
    /// 结构：两行注释头 → 六个分区 → 末尾身份标记行。
    /// `Runtime.Mode != Bundled` 时多写一行 `Path=`（模型有 12 键，文件行数 11 或 12）。
    pub fn render(&self, identity: &str) -> Result<String> {
        if let Some(custom) = &self.custom_ini {
            return Ok(render_custom(custom, identity));
        }
        self.validate()?;
        let mut text = String::with_capacity(512);
        text.push_str("; DlssgLauncher 生成的 dlssg_sm86 运行配置\r\n");
        text.push_str("; 请用启动器界面修改；直接改这个文件会在下次启动时被覆盖\r\n");
        text.push_str("\r\n[General]\r\n");
        text.push_str(&format!("Enabled={}\r\n", u8::from(self.enabled)));
        text.push_str("\r\n[FrameGeneration]\r\n");
        text.push_str(&format!("Engine={}\r\n", self.engine.as_str()));
        text.push_str(&format!(
            "MaxGeneratedFrames={}\r\n",
            self.max_generated_frames
        ));
        text.push_str("\r\n[DLSSG]\r\n");
        text.push_str(&format!("Preset={}\r\n", self.preset.as_str()));
        text.push_str(&format!("Optimized={}\r\n", self.optimized));
        text.push_str("\r\n[SuperResolution]\r\n");
        text.push_str(&format!("Mode={}\r\n", self.super_resolution.as_str()));
        text.push_str("\r\n[Logging]\r\n");
        text.push_str(&format!("Level={}\r\n", self.level));
        text.push_str(&format!("Directory={}\r\n", self.directory));
        text.push_str("\r\n[XeSSFG]\r\n");
        text.push_str(&format!(
            "FlowResolution={}\r\n",
            self.flow_resolution.as_str()
        ));
        text.push_str(&format!("UseHudless={}\r\n", self.use_hudless.as_str()));
        text.push_str("\r\n[Runtime]\r\n");
        text.push_str(&format!("Mode={}\r\n", self.runtime_mode.as_str()));
        if let Some(path) = &self.runtime_path {
            if self.runtime_mode.needs_path() {
                text.push_str(&format!("Path={}\r\n", path.trim()));
            }
        }
        text.push_str(&format!("\r\n{MARKER_PREFIX}{identity}\r\n"));
        Ok(text)
    }
}

/// 自定义 INI：用户文本原样保留，只在最前面插入标记行。
fn render_custom(custom: &str, identity: &str) -> String {
    let mut text = String::with_capacity(custom.len() + 64);
    text.push_str(MARKER_PREFIX);
    text.push_str(identity);
    text.push_str("\r\n");
    text.push_str(custom);
    text
}

/// 从生成文本里取回标记行里的身份。
pub fn marker_identity(text: &str) -> Option<&str> {
    text.lines()
        .find_map(|line| line.trim_end().strip_prefix(MARKER_PREFIX))
        .map(str::trim)
}

/// 已删除或已迁位的旧键提示（界面用）。
pub fn legacy_key_hints(text: &str) -> Vec<String> {
    let mut hints = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(';') || trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match (key, value) {
            ("Preset", "Auto") => {
                hints.push("Preset=Auto 已于 2026-10-22 删除，按 A 运行。".to_owned())
            }
            ("Mode", "Auto") => {
                hints.push("Runtime.Mode=Auto 已在 0.4.0 删除，按 Bundled 运行。".to_owned())
            }
            ("Model", _) => hints.push("Model 键已于 2026-10-26 删除，不再读取。".to_owned()),
            ("SRMode", _) | ("SRPath", _) => {
                hints.push(format!("{key} 是旧键名，请改用 [SuperResolution] Mode。"))
            }
            _ => {}
        }
    }
    hints.sort();
    hints.dedup();
    hints
}

/// 一次"导入完整 INI"的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub applied: Vec<String>,
    pub unknown: Vec<String>,
    pub hints: Vec<String>,
}

impl Settings {
    /// 把一份完整 INI 的内容合并进设置。无法识别的键记进 `unknown`，不报错。
    pub fn apply_ini(&mut self, text: &str) -> Result<ImportReport> {
        let document = config::parse(text)?;
        let mut report = ImportReport {
            hints: legacy_key_hints(text),
            ..ImportReport::default()
        };
        let mut custom_sections = 0_usize;
        for section in &document.sections {
            for entry in &section.entries {
                let applied = match (section.label.as_str(), entry.key.as_str()) {
                    ("General", "Enabled") => parse_flag(&entry.value).map(|value| {
                        self.enabled = value;
                    }),
                    ("FrameGeneration", "Engine") => EngineKind::parse(&entry.value).map(|value| {
                        self.engine = value;
                    }),
                    ("FrameGeneration", "MaxGeneratedFrames") => {
                        parse_u8(&entry.value, "MaxGeneratedFrames")
                            .map(|value| self.max_generated_frames = value)
                    }
                    ("DLSSG", "Preset") => Preset::parse(&entry.value).map(|value| {
                        self.preset = value;
                    }),
                    ("DLSSG", "Optimized") => {
                        parse_u8(&entry.value, "Optimized").map(|value| self.optimized = value)
                    }
                    ("SuperResolution", "Mode") => SuperResolutionMode::parse(&entry.value)
                        .map(|value| self.super_resolution = value),
                    ("Logging", "Level") => {
                        parse_u8(&entry.value, "Level").map(|value| self.level = value)
                    }
                    ("Logging", "Directory") => {
                        self.directory = entry.value.clone();
                        Ok(())
                    }
                    ("XeSSFG", "FlowResolution") => FlowResolution::parse(&entry.value)
                        .map(|value| self.flow_resolution = value),
                    ("XeSSFG", "UseHudless") => {
                        UseHudless::parse(&entry.value).map(|value| self.use_hudless = value)
                    }
                    ("Runtime", "Mode") => {
                        RuntimeMode::parse(&entry.value).map(|value| self.runtime_mode = value)
                    }
                    ("Runtime", "Path") => {
                        self.runtime_path = Some(entry.value.clone());
                        Ok(())
                    }
                    _ => {
                        custom_sections += 1;
                        report
                            .unknown
                            .push(format!("{}.{}", section.label, entry.key));
                        continue;
                    }
                };
                applied?;
                report
                    .applied
                    .push(format!("{}.{}", section.label, entry.key));
            }
        }
        if custom_sections > 0 {
            report.hints.push(format!(
                "{custom_sections} 个键不属于 12 键模型，已保留在原文中。"
            ));
        }
        Ok(report)
    }
}

fn parse_flag(value: &str) -> Result<bool> {
    match value.trim() {
        "1" => Ok(true),
        "0" => Ok(false),
        other => Err(Error::invalid(format!("Enabled 只能取 0 或 1：{other}"))),
    }
}

fn parse_u8(value: &str, name: &str) -> Result<u8> {
    value
        .trim()
        .parse()
        .map_err(|_| Error::invalid(format!("{name} 必须是数字：{value}")))
}

// ---------------------------------------------------------------- 诊断模板

/// 生成临时诊断 INI，等价 `Set-LauncherDiagnosticOptions`。
///
/// 纯字面替换，顺序固定：`Optimized` → `Level` → `Preset` → `Mode`（LF 与 CRLF 两种写法）。
/// 顺序不能改：模板里同时存在 `Optimized=2` 与 `Level=2`。
pub fn render_diagnostic(
    template: &str,
    mode: SuperResolutionMode,
    preset: Preset,
    optimized: u8,
    level: u8,
) -> String {
    template
        .replace("Optimized=2", &format!("Optimized={optimized}"))
        .replace("Level=2", &format!("Level={level}"))
        .replace("Preset=A", &format!("Preset={}", preset.as_str()))
        .replace(
            "[SuperResolution]\nMode=Off",
            &format!("[SuperResolution]\nMode={}", mode.as_str()),
        )
        .replace(
            "[SuperResolution]\r\nMode=Off",
            &format!("[SuperResolution]\r\nMode={}", mode.as_str()),
        )
}

/// 从诊断文本里取回四个可配置值。等价 `Test-LauncherDiagnosticOptionsText` 的提取部分。
pub fn extract_diagnostic_values(contents: &str) -> Option<(SuperResolutionMode, Preset, u8, u8)> {
    let mut mode = None;
    let mut preset = None;
    let mut optimized = None;
    let mut level = None;
    for line in contents.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(value) = line.strip_prefix("Mode=") {
            if mode.is_some() {
                return None;
            }
            mode = SuperResolutionMode::parse(value).ok();
        } else if let Some(value) = line.strip_prefix("Preset=") {
            if preset.is_some() {
                return None;
            }
            preset = Preset::parse(value).ok();
        } else if let Some(value) = line.strip_prefix("Optimized=") {
            if optimized.is_some() {
                return None;
            }
            optimized = parse_range_0_3(value);
        } else if let Some(value) = line.strip_prefix("Level=") {
            if level.is_some() {
                return None;
            }
            level = parse_range_0_3(value);
        }
    }
    Some((mode?, preset?, optimized?, level?))
}

/// 只接受 0–3，等价 PS 正则 `Optimized=([0-3])` 的约束。
fn parse_range_0_3(value: &str) -> Option<u8> {
    let parsed: u8 = value.trim().parse().ok()?;
    (parsed <= 3).then_some(parsed)
}

/// 一个入口的诊断识别集合：可再生成的 0.4.2 模板 + 必须逐字匹配的旧文本。
///
/// 现有实现里旧文本是 `-ceq` 直接比较（`Test-StaleDiagnosticIniTarget`），
/// 并不经过"提取→重生成"，所以这里把两类分开。
#[derive(Debug, Clone)]
pub struct DiagnosticAcceptance {
    pub template: String,
    pub legacy_exact: Vec<String>,
}

impl DiagnosticAcceptance {
    pub fn new(template: impl Into<String>, legacy_exact: Vec<String>) -> Self {
        Self {
            template: template.into(),
            legacy_exact,
        }
    }

    pub fn accepts(&self, contents: &str) -> bool {
        self.legacy_exact.iter().any(|legacy| legacy == contents)
            || recognizes_diagnostic(&self.template, contents)
    }
}

/// 整段文本是否与"从该模板重新生成"完全一致（`-ceq` 语义）。
pub fn recognizes_diagnostic(template: &str, contents: &str) -> bool {
    let Some((mode, preset, optimized, level)) = extract_diagnostic_values(contents) else {
        return false;
    };
    render_diagnostic(template, mode, preset, optimized, level) == contents
}

/// 任一旧模板能重新生成这段文本即视为本启动器的产物（无记录回收用）。
pub fn recognizes_any_diagnostic(templates: &[String], contents: &str) -> bool {
    templates
        .iter()
        .any(|template| recognizes_diagnostic(template, contents))
}

/// 诊断临时文件名：`<prefix><pid>.ini`。
pub fn is_diagnostic_file_name(prefix: &str, file_name: &str) -> bool {
    let Some(rest) = file_name.strip_prefix(prefix) else {
        return false;
    };
    let Some(digits) = rest.strip_suffix(".ini") else {
        return false;
    };
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_0_4_2_release_configuration() {
        let settings = Settings::default();
        assert!(settings.enabled);
        assert_eq!(settings.engine, EngineKind::Dlss);
        assert_eq!(settings.max_generated_frames, 3);
        assert_eq!(settings.preset, Preset::A);
        assert_eq!(settings.optimized, 1);
        assert_eq!(settings.level, 1);
        assert_eq!(settings.super_resolution, SuperResolutionMode::Off);
        assert_eq!(settings.runtime_mode, RuntimeMode::Bundled);
        assert!(settings.runtime_path.is_none());
        assert!(settings.custom_ini.is_none());
        settings.validate().expect("出厂值应合法");
    }

    #[test]
    fn rendering_is_byte_stable_and_carries_the_marker() {
        let settings = Settings::default();
        let first = settings.render("0123456789abcdef").expect("渲染");
        let second = settings.render("0123456789abcdef").expect("渲染");
        assert_eq!(first, second, "同样设置必须逐字节相同");
        assert!(first.contains("\r\n"));
        assert!(
            !first.contains('\n') || first.matches("\r\n").count() == first.matches('\n').count()
        );
        assert!(first.ends_with(&format!("{MARKER_PREFIX}0123456789abcdef\r\n")));
        assert_eq!(marker_identity(&first), Some("0123456789abcdef"));
        assert!(first.contains("MaxGeneratedFrames=3"));
        assert!(first.contains("Optimized=1"));
        assert!(first.contains("Level=1"));
        assert!(first.contains("[Runtime]\r\nMode=Bundled\r\n"));
        assert!(!first.contains("Path="), "Bundled 不应写 Path");
    }

    #[test]
    fn external_runtime_adds_the_path_line() {
        let external = Settings {
            runtime_mode: RuntimeMode::External,
            runtime_path: Some(r"D:\runtime\nvngx_dlssg.dll".to_owned()),
            ..Settings::default()
        };
        external.validate().expect("合法");
        let text = external.render("id").expect("渲染");
        assert!(text.contains("Mode=External\r\nPath=D:\\runtime\\nvngx_dlssg.dll\r\n"));

        let bundled_with_path = Settings {
            runtime_path: Some(r"D:\runtime\nvngx_dlssg.dll".to_owned()),
            ..Settings::default()
        };
        assert!(
            bundled_with_path.validate().is_err(),
            "Bundled 带 Path 应报错"
        );
    }

    #[test]
    fn custom_ini_is_written_verbatim_with_marker_first() {
        let settings = Settings {
            custom_ini: Some("[General]\r\nEnabled=0\r\n".to_owned()),
            max_generated_frames: 99, // 自定义模式不校验结构化字段
            ..Settings::default()
        };
        let text = settings.render("abc").expect("渲染");
        assert!(text.starts_with(&format!("{MARKER_PREFIX}abc\r\n")));
        assert!(text.ends_with("[General]\r\nEnabled=0\r\n"));
        assert!(settings.render("abc").is_ok());
    }

    #[test]
    fn validation_covers_ranges_and_cross_rules() {
        let too_many = Settings {
            max_generated_frames: 6,
            ..Settings::default()
        };
        assert!(too_many.validate().unwrap_err().to_string().contains("1–5"));

        let xess_too_many = Settings {
            engine: EngineKind::XeSs,
            max_generated_frames: 4,
            ..Settings::default()
        };
        assert!(
            xess_too_many
                .validate()
                .unwrap_err()
                .to_string()
                .contains("1–3")
        );

        Settings {
            engine: EngineKind::XeSs,
            max_generated_frames: 3,
            ..Settings::default()
        }
        .validate()
        .expect("XeSS 3 帧合法");

        assert!(
            Settings {
                optimized: 4,
                ..Settings::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            Settings {
                level: 9,
                ..Settings::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            Settings {
                directory: "  ".to_owned(),
                ..Settings::default()
            }
            .validate()
            .is_err()
        );
        Settings::default().validate().expect("出厂值合法");
    }

    #[test]
    fn super_resolution_flag_drives_resource_requirements() {
        assert!(!Settings::default().requires_super_resolution_resources());
        let supersampled = Settings {
            super_resolution: SuperResolutionMode::LInt8,
            ..Settings::default()
        };
        assert!(supersampled.requires_super_resolution_resources());
    }

    #[test]
    fn legacy_keys_produce_hints() {
        let text = "[DLSSG]\r\nPreset=Auto\r\nModel=15\r\n[Runtime]\r\nMode=Auto\r\n";
        let hints = legacy_key_hints(text);
        assert!(
            hints.iter().any(|hint| hint.contains("Preset=Auto")),
            "{hints:?}"
        );
        assert!(hints.iter().any(|hint| hint.contains("Model")), "{hints:?}");
        assert!(
            hints.iter().any(|hint| hint.contains("Runtime.Mode")),
            "{hints:?}"
        );
        assert!(legacy_key_hints("[DLSSG]\r\nPreset=A\r\n").is_empty());
    }

    #[test]
    fn diagnostic_file_names_are_prefix_plus_digits() {
        assert!(is_diagnostic_file_name(
            "dlssg_sm86-wuthering-",
            "dlssg_sm86-wuthering-1234.ini"
        ));
        assert!(!is_diagnostic_file_name(
            "dlssg_sm86-wuthering-",
            "dlssg_sm86-wuthering-.ini"
        ));
        assert!(!is_diagnostic_file_name(
            "dlssg_sm86-wuthering-",
            "dlssg_sm86-nte-1.ini"
        ));
        assert!(!is_diagnostic_file_name(
            "dlssg_sm86-wuthering-",
            "dlssg_sm86-wuthering-12.txt"
        ));
    }
}
