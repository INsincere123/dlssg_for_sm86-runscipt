//! `启动配置.ini` 的读取（Phase 1 步 3）。
//!
//! 对应现有实现 `LauncherConfig.ps1:6-120`：
//! - 编码识别顺序：UTF-32 LE/BE → UTF-8 BOM → UTF-16 LE/BE → UTF-8（失败再按 GBK 回退并告警）
//! - 分行：`\r\n` / `\n` / `\r` 三种都接受
//! - 注释：整行 `;` 或 `#`；**不切分行尾注释**（`#`、`;`、`=` 都是合法文件名字符）
//! - 分组：`[名称]`，中文分组名有别名（鸣潮 / 异环 / 显卡）
//! - 键：`键=值`，中文键有英文别名；同一分组内重复键报错；分组重复报错
//! - 错误带 1 起始行号
//!
//! 本模块只负责"读成结构化数据"，不校验某个分组允许哪些键（那是各入口的档案层职责）。

use std::path::Path;

use crate::error::{Error, Result};

/// 识别出的文件编码。写回时（步 5）用它决定输出的字节编码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEncoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Utf32Le,
    Utf32Be,
    /// 非 UTF-8 且无 BOM，按 GBK/CP936 解读。现有实现会给出告警。
    Gbk,
}

impl TextEncoding {
    pub fn label(self) -> &'static str {
        match self {
            TextEncoding::Utf8 => "utf-8",
            TextEncoding::Utf8Bom => "utf-8-bom",
            TextEncoding::Utf16Le => "utf-16-le",
            TextEncoding::Utf16Be => "utf-16-be",
            TextEncoding::Utf32Le => "utf-32-le",
            TextEncoding::Utf32Be => "utf-32-be",
            TextEncoding::Gbk => "gbk",
        }
    }
}

/// 解码结果。`bom` 原样保留，供写回时还原。
#[derive(Debug, Clone)]
pub struct DecodedText {
    pub text: String,
    pub encoding: TextEncoding,
    pub bom: Vec<u8>,
    pub warnings: Vec<String>,
}

impl DecodedText {
    /// 写回时是否要带 BOM。
    pub fn writes_bom(&self) -> bool {
        !self.bom.is_empty()
    }
}

/// 按现有实现的顺序识别编码并解码。
pub fn decode(bytes: &[u8]) -> Result<DecodedText> {
    // 顺序不能变：UTF-32 LE 的 BOM 以 FF FE 开头，必须早于 UTF-16 LE 判断。
    if bytes.starts_with(&[0xFF, 0xFE, 0x00, 0x00]) {
        return decode_utf32(
            &bytes[4..],
            true,
            TextEncoding::Utf32Le,
            bytes[..4].to_vec(),
        );
    }
    if bytes.starts_with(&[0x00, 0x00, 0xFE, 0xFF]) {
        return decode_utf32(
            &bytes[4..],
            false,
            TextEncoding::Utf32Be,
            bytes[..4].to_vec(),
        );
    }
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return decode_utf8(&bytes[3..], TextEncoding::Utf8Bom, bytes[..3].to_vec());
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return decode_utf16(
            &bytes[2..],
            true,
            TextEncoding::Utf16Le,
            bytes[..2].to_vec(),
        );
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return decode_utf16(
            &bytes[2..],
            false,
            TextEncoding::Utf16Be,
            bytes[..2].to_vec(),
        );
    }

    // 没有 BOM：先严格按 UTF-8，失败再按 GBK 并告警（现有实现的行为）。
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(DecodedText {
            text: text.to_owned(),
            encoding: TextEncoding::Utf8,
            bom: Vec::new(),
            warnings: Vec::new(),
        }),
        Err(_) => {
            let (decoded, _, had_errors) = encoding_rs::GBK.decode(bytes);
            let mut warnings = vec![
                "启动配置不是有效 UTF-8，已按 GBK/CP936 尝试读取；请确认中文正确，建议另存为 UTF-8。"
                    .to_owned(),
            ];
            if had_errors {
                warnings.push("GBK 解码存在无法映射的字节，文件可能已损坏。".to_owned());
            }
            Ok(DecodedText {
                text: decoded.into_owned(),
                encoding: TextEncoding::Gbk,
                bom: Vec::new(),
                warnings,
            })
        }
    }
}

fn decode_utf8(body: &[u8], encoding: TextEncoding, bom: Vec<u8>) -> Result<DecodedText> {
    let text = std::str::from_utf8(body)
        .map_err(|_| Error::invalid(format!("{} 文件不是合法 UTF-8", encoding.label())))?;
    Ok(DecodedText {
        text: text.to_owned(),
        encoding,
        bom,
        warnings: Vec::new(),
    })
}

fn decode_utf16(
    body: &[u8],
    little_endian: bool,
    encoding: TextEncoding,
    bom: Vec<u8>,
) -> Result<DecodedText> {
    if body.len() % 2 != 0 {
        return Err(Error::invalid(format!(
            "{} 文件长度为奇数，无法按 UTF-16 解析",
            encoding.label()
        )));
    }
    let units: Vec<u16> = body
        .chunks_exact(2)
        .map(|pair| {
            if little_endian {
                u16::from_le_bytes([pair[0], pair[1]])
            } else {
                u16::from_be_bytes([pair[0], pair[1]])
            }
        })
        .collect();
    let text = String::from_utf16(&units)
        .map_err(|_| Error::invalid(format!("{} 文件含未配对的代理项", encoding.label())))?;
    Ok(DecodedText {
        text,
        encoding,
        bom,
        warnings: Vec::new(),
    })
}

fn decode_utf32(
    body: &[u8],
    little_endian: bool,
    encoding: TextEncoding,
    bom: Vec<u8>,
) -> Result<DecodedText> {
    if body.len() % 4 != 0 {
        return Err(Error::invalid(format!(
            "{} 文件长度不是 4 的倍数，无法按 UTF-32 解析",
            encoding.label()
        )));
    }
    let mut text = String::with_capacity(body.len() / 4);
    for chunk in body.chunks_exact(4) {
        let value = if little_endian {
            u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
        } else {
            u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
        };
        let character = char::from_u32(value).ok_or_else(|| {
            Error::invalid(format!("{} 文件含非法码位 U+{value:04X}", encoding.label()))
        })?;
        text.push(character);
    }
    Ok(DecodedText {
        text,
        encoding,
        bom,
        warnings: Vec::new(),
    })
}

/// 一个 `键=值` 条目。`key_raw` 保留文件里的原文，`key` 是别名解析后的规范名。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub line: usize,
    pub key_raw: String,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub line: usize,
    pub label_raw: String,
    /// 别名解析后的分组名，例如 `鸣潮` → `Wuthering`。
    pub label: String,
    pub entries: Vec<Entry>,
}

impl Section {
    pub fn get(&self, key: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.key == key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub sections: Vec<Section>,
}

impl Document {
    pub fn section(&self, label: &str) -> Option<&Section> {
        let wanted = canonical_group(label);
        self.sections.iter().find(|section| section.label == wanted)
    }
}

/// 分组名别名。未列出的名称原样返回（Steam / SteamUE5:xxx / DirectUE:xxx 等动态分组）。
pub fn canonical_group(label: &str) -> String {
    match label {
        "鸣潮" => "Wuthering",
        "异环" => "Nte",
        "显卡" => "Gpu",
        other => other,
    }
    .to_owned()
}

/// 键名别名。未列出的键原样返回。
pub fn canonical_key(key: &str) -> String {
    match key {
        "游戏目录" => "GameDirectory",
        "启动器路径" => "LauncherPath",
        "配置源" => "EngineConfigSource",
        "游戏配置目录" => "ConfigDirectory",
        "重启等待秒数" => "RestartGraceSeconds",
        "设备实例标识" => "DeviceInstanceId",
        "目标显示名称" => "TargetName",
        "启动程序" => "LaunchExecutable",
        "游戏名称" => "GameName",
        "渲染程序" => "RenderExecutable",
        "代理DLL" => "ProxyDll",
        other => other,
    }
    .to_owned()
}

/// 解析整份文本。
pub fn parse(text: &str) -> Result<Document> {
    if text.contains('\0') || text.contains('\u{FFFD}') {
        return Err(Error::config_plain(
            "启动配置含无效文本（NUL 或替换字符），请另存为 UTF-8。",
        ));
    }

    let mut sections: Vec<Section> = Vec::new();
    for (index, raw_line) in split_lines(text).iter().enumerate() {
        let line_number = index + 1;
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
            continue;
        }

        if let Some(label_raw) = parse_header(trimmed) {
            let label = canonical_group(&label_raw);
            if sections.iter().any(|section| section.label == label) {
                return Err(Error::config(line_number, format!("分组 {label} 重复。")));
            }
            sections.push(Section {
                line: line_number,
                label_raw,
                label,
                entries: Vec::new(),
            });
            continue;
        }

        let separator = trimmed.find('=');
        let Some(separator) = separator else {
            return Err(Error::config(
                line_number,
                "格式错误，应为 [分组] 或 键=值。",
            ));
        };
        if separator == 0 || sections.is_empty() {
            return Err(Error::config(
                line_number,
                "格式错误，应为 [分组] 或 键=值。",
            ));
        }

        let key_raw = trimmed[..separator].trim().to_owned();
        let key = canonical_key(&key_raw);
        let value = trimmed[separator + 1..].trim().to_owned();
        let section = sections.last_mut().expect("上面已确认非空");
        if section.entries.iter().any(|entry| entry.key == key) {
            return Err(Error::config(
                line_number,
                format!("配置项 {}.{key} 重复。", section.label),
            ));
        }
        section.entries.push(Entry {
            line: line_number,
            key_raw,
            key,
            value,
        });
    }

    Ok(Document { sections })
}

/// 解码 + 解析。
pub fn parse_bytes(bytes: &[u8]) -> Result<(Document, DecodedText)> {
    let decoded = decode(bytes)?;
    let document = parse(&decoded.text)?;
    Ok((document, decoded))
}

/// 读文件并解析。错误里会带上路径。
pub fn read_file(path: &Path) -> Result<(Document, DecodedText)> {
    let bytes = std::fs::read(path).map_err(|error| Error::io(path, error))?;
    parse_bytes(&bytes)
}

fn parse_header(trimmed: &str) -> Option<String> {
    let inner = trimmed.strip_prefix('[')?.strip_suffix(']')?;
    if inner.is_empty() || inner.contains('[') || inner.contains(']') {
        return None;
    }
    Some(inner.trim().to_owned())
}

/// 等价 `-split '\r\n|\n|\r'`：三种换行都当成一个分隔符。
fn split_lines(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' => {
                lines.push(&text[start..index]);
                index += if index + 1 < bytes.len() && bytes[index + 1] == b'\n' {
                    2
                } else {
                    1
                };
                start = index;
            }
            b'\n' => {
                lines.push(&text[start..index]);
                index += 1;
                start = index;
            }
            _ => index += 1,
        }
    }
    lines.push(&text[start..]);
    lines
}

/// 去掉外层引号。支持 `"`、`'` 与中文弯引号，与 `Remove-LauncherValueQuotes` 一致。
pub fn unquote(value: &str, name: &str) -> Result<String> {
    let trimmed = value.trim();
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return Ok(String::new());
    };
    let closer = match first {
        '"' => Some('"'),
        '\'' => Some('\''),
        '\u{201C}' => Some('\u{201D}'),
        '\u{2018}' => Some('\u{2019}'),
        _ => None,
    };
    let Some(closer) = closer else {
        return Ok(trimmed.to_owned());
    };
    if trimmed.chars().count() < 2 || !trimmed.ends_with(closer) {
        return Err(Error::invalid(format!(
            "启动配置 {name} 的外层引号不成对。"
        )));
    }
    Ok(trimmed[first.len_utf8()..trimmed.len() - closer.len_utf8()].to_owned())
}

/// 路径值解析环境（Phase 1 步 4）。
///
/// 对应 `Resolve-LauncherConfigPath`（`LauncherConfig.ps1:122-168`）：
/// 去引号 → file URI → 环境变量展开 → `~` → 去掉 `FileSystem::` 前缀 →
/// 统一分隔符 → 处理 `\\?\` 前缀 → 拒绝盘符相对路径与非法字符 →
/// 相对路径拼到启动器根目录 → 取全路径。
///
/// 环境变量与 home 都靠注入，测试不需要改真实进程环境。
pub struct PathEnvironment<'a> {
    pub root: &'a Path,
    pub home: Option<&'a Path>,
    pub get_env: &'a dyn Fn(&str) -> Option<String>,
}

impl PathEnvironment<'_> {
    /// 解析一个配置值里的路径。空值返回空串（调用方据此走"自动发现"分支）。
    pub fn resolve(&self, raw: &str, name: &str) -> Result<String> {
        if raw.trim().is_empty() {
            return Ok(String::new());
        }
        let unquoted = unquote(raw, name)?;

        let mut expanded = match parse_file_uri(&unquoted, name)? {
            Some(local) => local,
            None => expand_environment(&unquoted, name, self.get_env)?,
        };

        // `~` 与 `~/x` 都展开到用户目录。
        if expanded == "~" || expanded.starts_with("~\\") || expanded.starts_with("~/") {
            let home = self.home.ok_or_else(|| {
                Error::invalid(format!("启动配置 {name} 使用了 ~，但无法确定用户目录。"))
            })?;
            let rest = expanded[1..].trim_start_matches(['\\', '/']);
            let base = home.to_string_lossy().replace('/', "\\");
            expanded = if rest.is_empty() {
                base
            } else {
                format!("{}\\{rest}", base.trim_end_matches('\\'))
            };
        }

        expanded = strip_filesystem_prefix(&expanded);
        expanded = expanded.replace('/', "\\");

        // 扩展路径前缀统一成一种写法，避免 journal identity 与互斥名不一致。
        if starts_with_ignore_case(&expanded, "\\\\?\\UNC\\") {
            expanded = format!("\\\\{}", &expanded[8..]);
        } else if is_extended_drive_path(&expanded) {
            expanded = expanded[4..].to_owned();
        } else if expanded.starts_with("\\\\?\\") || expanded.starts_with("\\\\.\\") {
            return Err(Error::invalid(format!(
                "启动配置 {name} 只支持普通文件路径、盘符或 UNC 扩展路径：{raw}"
            )));
        }

        if is_drive_relative(&expanded) {
            return Err(Error::invalid(format!(
                "启动配置 {name} 不支持含义不明确的盘符相对路径，请写 C:\\目录 或相对路径：{raw}"
            )));
        }
        if expanded
            .chars()
            .any(|character| character.is_control() || "<>\"|?*".contains(character))
        {
            return Err(Error::invalid(format!(
                "启动配置 {name} 含无效路径字符：{raw}"
            )));
        }

        // 单个反斜杠开头：拼当前盘根；否则相对路径拼启动器根。
        if expanded.starts_with('\\') && !expanded.starts_with("\\\\") {
            expanded = format!("{}{expanded}", root_prefix(self.root)?);
        } else if !is_rooted(&expanded) {
            let base = self.root.to_string_lossy().replace('/', "\\");
            expanded = format!("{}\\{expanded}", base.trim_end_matches('\\'));
        }

        let context = crate::path::PathContext::with_working_directory(self.root);
        context
            .full_path(&expanded)
            .map_err(|error| Error::invalid(format!("启动配置 {name} 路径无效：{raw}；{error}")))
    }
}

fn starts_with_ignore_case(value: &str, prefix: &str) -> bool {
    // 必须按字符边界切片：前缀是 ASCII，值可能是中文路径。
    value
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

fn is_extended_drive_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 7
        && bytes.starts_with(b"\\\\?\\")
        && bytes[4].is_ascii_alphabetic()
        && bytes[5] == b':'
        && bytes[6] == b'\\'
}

fn is_drive_relative(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 2 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' {
        return false;
    }
    bytes.len() == 2 || bytes[2] != b'\\'
}

fn is_rooted(value: &str) -> bool {
    let bytes = value.as_bytes();
    (bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\')
        || value.starts_with("\\\\")
}

/// 启动器根所在的盘根，尾部分隔符已去掉（`D:\` → `D:`）。
fn root_prefix(root: &Path) -> Result<String> {
    let text = root.to_string_lossy().replace('/', "\\");
    let bytes = text.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Ok(text[..2].to_owned());
    }
    if let Some(rest) = text.strip_prefix("\\\\") {
        let mut parts = rest.splitn(3, '\\');
        let server = parts.next().unwrap_or_default();
        let share = parts.next().unwrap_or_default();
        if !server.is_empty() && !share.is_empty() {
            return Ok(format!("\\\\{server}\\{share}"));
        }
    }
    Err(Error::invalid(format!(
        "启动器根目录没有盘符或共享名，无法解析以 \\ 开头的路径：{}",
        root.display()
    )))
}

fn strip_filesystem_prefix(value: &str) -> String {
    for prefix in ["Microsoft.PowerShell.Core\\FileSystem::", "FileSystem::"] {
        if starts_with_ignore_case(value, prefix) {
            return value[prefix.len()..].to_owned();
        }
    }
    value.to_owned()
}

/// 识别 file URI。返回 `None` 表示"不是 URI"，交给环境变量展开。
fn parse_file_uri(value: &str, name: &str) -> Result<Option<String>> {
    let has_scheme = {
        let bytes = value.as_bytes();
        let mut index = 0;
        if bytes.first().is_some_and(u8::is_ascii_alphabetic) {
            index = 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric()
                    || matches!(bytes[index], b'+' | b'-' | b'.'))
            {
                index += 1;
            }
        }
        index > 0 && bytes.len() > index + 2 && &bytes[index..index + 3] == b"://"
    };
    let looks_like_file = starts_with_ignore_case(value, "file:");
    if !has_scheme && !looks_like_file {
        return Ok(None);
    }

    let scheme_end = value
        .find(':')
        .ok_or_else(|| Error::invalid(format!("启动配置 {name} 的 URI 缺少冒号：{value}")))?;
    if !value[..scheme_end].eq_ignore_ascii_case("file") {
        return Err(Error::invalid(format!(
            "启动配置 {name} 仅支持文件路径或 file URI，不支持此网址：{value}"
        )));
    }
    if value.contains('?') || value.contains('#') {
        return Err(Error::invalid(format!(
            "启动配置 {name} 的 file URI 不支持查询串或片段：{value}"
        )));
    }

    let rest = &value[scheme_end + 1..];
    let path = match rest.strip_prefix("//") {
        Some(after) => match after.strip_prefix('/') {
            Some(local) => local.to_owned(),
            None => format!("\\\\{after}"),
        },
        None => rest.to_owned(),
    };
    Ok(Some(percent_decode(&path)))
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = (bytes[index + 1] as char).to_digit(16);
            let low = (bytes[index + 2] as char).to_digit(16);
            if let (Some(high), Some(low)) = (high, low) {
                out.push((high * 16 + low) as u8);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 展开 `%NAME%`、`${env:NAME}`、`$env:NAME`（大小写不敏感）。
/// 未定义的变量按现有实现直接报错，不做静默替换。
fn expand_environment(
    value: &str,
    name: &str,
    get_env: &dyn Fn(&str) -> Option<String>,
) -> Result<String> {
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::with_capacity(value.len());
    let mut index = 0;
    while index < chars.len() {
        let current = chars[index];
        if current == '%' {
            if let Some(offset) = chars[index + 1..].iter().position(|&item| item == '%') {
                let variable: String = chars[index + 1..index + 1 + offset].iter().collect();
                if !variable.is_empty() {
                    out.push_str(&lookup_env(&variable, name, get_env)?);
                    index += offset + 2;
                    continue;
                }
            }
        } else if current == '$' {
            if let Some((variable, consumed)) = read_braced_env(&chars[index..]) {
                out.push_str(&lookup_env(&variable, name, get_env)?);
                index += consumed;
                continue;
            }
            if let Some((variable, consumed)) = read_plain_env(&chars[index..]) {
                out.push_str(&lookup_env(&variable, name, get_env)?);
                index += consumed;
                continue;
            }
        }
        out.push(current);
        index += 1;
    }
    Ok(out)
}

fn lookup_env(
    variable: &str,
    name: &str,
    get_env: &dyn Fn(&str) -> Option<String>,
) -> Result<String> {
    get_env(variable)
        .ok_or_else(|| Error::invalid(format!("启动配置 {name} 包含未定义的环境变量：{variable}")))
}

/// `${env:NAME}` → (NAME, 消耗的字符数)
fn read_braced_env(chars: &[char]) -> Option<(String, usize)> {
    const PREFIX: &[char] = &['$', '{', 'e', 'n', 'v', ':'];
    if chars.len() < PREFIX.len() + 1 {
        return None;
    }
    for (index, expected) in PREFIX.iter().enumerate() {
        if !chars[index].eq_ignore_ascii_case(expected) {
            return None;
        }
    }
    let mut end = PREFIX.len();
    while end < chars.len() && chars[end] != '}' {
        end += 1;
    }
    if end >= chars.len() || end == PREFIX.len() {
        return None;
    }
    Some((chars[PREFIX.len()..end].iter().collect(), end + 1))
}

/// `$env:NAME` → (NAME, 消耗的字符数)
fn read_plain_env(chars: &[char]) -> Option<(String, usize)> {
    const PREFIX: &[char] = &['$', 'e', 'n', 'v', ':'];
    if chars.len() <= PREFIX.len() {
        return None;
    }
    for (index, expected) in PREFIX.iter().enumerate() {
        if !chars[index].eq_ignore_ascii_case(expected) {
            return None;
        }
    }
    let first = chars[PREFIX.len()];
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    let mut end = PREFIX.len() + 1;
    while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_') {
        end += 1;
    }
    Some((chars[PREFIX.len()..end].iter().collect(), end))
}

/// 写回结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome {
    /// 键已有非空值：现有实现**不覆盖**，直接返回原值（由调用方解析）。
    AlreadySet(String),
    /// 已写入（新增或替换那一行）。
    Written,
}

/// 一次"记录检测到的路径"的请求，对应 `Save-LauncherDetectedPath`。
pub struct WriteRequest<'a> {
    pub path: &'a Path,
    /// 文件里使用的分组名（`鸣潮` / `异环`）。
    pub group_label: &'a str,
    /// 规范分组名（`Wuthering` / `Nte`），用于别名匹配与错误信息。
    pub group_id: &'a str,
    /// 新增行时使用的中文键名。
    pub display_key: &'a str,
    /// 英文键名（`GameDirectory` / `ConfigDirectory`）。
    pub english_key: &'a str,
    pub value: &'a str,
}

/// 在互斥保护下把检测到的路径写回 `启动配置.ini`，保留原编码与 BOM。
///
/// 与现有实现的差异只有一处：写回前的"文件是否被改动"检查使用字节比对，
/// 而不是 PowerShell 的 Base64 字符串比对（结果等价）。
pub fn write_if_absent(
    ops: &dyn crate::ops::MutexOps,
    wait: std::time::Duration,
    request: &WriteRequest<'_>,
) -> Result<WriteOutcome> {
    let path_text = request
        .path
        .to_str()
        .ok_or_else(|| Error::invalid(format!("路径不是合法 UTF-8：{}", request.path.display())))?;
    let context = crate::path::PathContext::current()?;
    let identity =
        crate::path::sha256_upper(context.full_path(path_text)?.to_lowercase().as_bytes());
    let mutex_name = format!("Local\\DLSSG-LauncherConfig-{identity}");

    let lease = crate::ops::MutexLease::acquire(ops, &mutex_name, wait)?;
    let result = write_locked(request);
    let release = lease.release();
    match (result, release) {
        (Ok(outcome), Ok(())) => Ok(outcome),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

fn write_locked(request: &WriteRequest<'_>) -> Result<WriteOutcome> {
    let metadata =
        std::fs::symlink_metadata(request.path).map_err(|error| Error::io(request.path, error))?;
    if metadata.file_type().is_symlink() {
        return Err(Error::invalid(
            "配置是链接，请手动填写对应路径。".to_owned(),
        ));
    }
    if metadata.permissions().readonly() {
        return Err(Error::invalid(
            "配置文件为只读，请取消只读后重试或手动填写。".to_owned(),
        ));
    }
    let original = std::fs::read(request.path).map_err(|error| Error::io(request.path, error))?;
    let decoded = decode(&original)?;
    let document = parse(&decoded.text)?;

    let labels = [request.group_label, request.group_id];
    let existing = document
        .sections
        .iter()
        .filter(|section| {
            labels.contains(&section.label_raw.as_str()) || labels.contains(&section.label.as_str())
        })
        .flat_map(|section| section.entries.iter())
        .find(|entry| entry.key == request.english_key)
        .map(|entry| entry.value.clone());
    if let Some(value) = existing {
        if !value.trim().is_empty() {
            return Ok(WriteOutcome::AlreadySet(unquote(
                &value,
                request.english_key,
            )?));
        }
    }

    let updated = replace_or_insert(&decoded.text, request)?;
    let bytes = encode(&decoded, &updated)?;
    let temporary = temporary_sibling(request.path);

    std::fs::write(&temporary, &bytes).map_err(|error| Error::io(&temporary, error))?;
    // 写回前确认文件没有被别人改过（现有实现用 Base64 比对做同一件事）。
    let current = std::fs::read(request.path).map_err(|error| Error::io(request.path, error))?;
    if current != original {
        let _ = std::fs::remove_file(&temporary);
        return Err(Error::invalid(
            "配置在更新过程中被修改，已停止写回，请重试。".to_owned(),
        ));
    }
    std::fs::rename(&temporary, request.path).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        Error::io(request.path, error)
    })?;
    Ok(WriteOutcome::Written)
}

/// 替换已有键那一行；没有该键时插到分组标题行后面。
fn replace_or_insert(text: &str, request: &WriteRequest<'_>) -> Result<String> {
    let mut offset = 0usize;
    let mut in_section = false;
    let mut found_section_line = false;
    let mut replacement: Option<(usize, usize, String)> = None;
    let mut insert_at: Option<(usize, bool)> = None; // (偏移, 该行是否已带换行)

    for line in split_lines_with_ends(text) {
        let start = offset;
        let end = offset + line.len();
        offset = end;

        let trimmed = line.trim_end_matches(['\r', '\n']).trim();
        if let Some(label) = parse_header(trimmed) {
            let matched = label == request.group_label || label == request.group_id;
            in_section = matched;
            if matched {
                found_section_line = true;
                insert_at = Some((end, line.ends_with('\n') || line.ends_with('\r')));
            }
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some(prefix_end) = match_key_prefix(line, request) {
            let content_len = line.trim_end_matches(['\r', '\n']).len();
            replacement = Some((
                start + prefix_end,
                start + content_len,
                request.value.to_owned(),
            ));
        }
    }

    if !found_section_line {
        return Err(Error::invalid(format!(
            "找不到配置分组：{}",
            request.group_id
        )));
    }

    if let Some((begin, end, value)) = replacement {
        let mut updated = String::with_capacity(text.len() + value.len());
        updated.push_str(&text[..begin]);
        updated.push_str(&value);
        updated.push_str(&text[end..]);
        return Ok(updated);
    }

    let (at, has_newline) = insert_at.expect("已确认找到分组标题行");
    let newline = detect_newline(text);
    let mut inserted = String::new();
    if !has_newline {
        inserted.push_str(newline);
    }
    inserted.push_str(request.display_key);
    inserted.push('=');
    inserted.push_str(request.value);
    inserted.push_str(newline);

    let mut updated = String::with_capacity(text.len() + inserted.len());
    updated.push_str(&text[..at]);
    updated.push_str(&inserted);
    updated.push_str(&text[at..]);
    Ok(updated)
}

/// 找出 `键=` 之后的字节位置（等价 `^(\s*(?:中文|Key)\s*=)` 的 group 1 结尾）。
/// 替换时必须保留键名，只替换等号后面的值。
fn match_key_prefix(line: &str, request: &WriteRequest<'_>) -> Option<usize> {
    let trimmed = line.trim_end_matches(['\r', '\n']);
    let eq = trimmed.find('=')?;
    let key = trimmed[..eq].trim();
    if key != request.display_key && key != request.english_key {
        return None;
    }
    Some(eq + 1)
}

fn detect_newline(text: &str) -> &'static str {
    let bytes = text.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        match byte {
            b'\r' => {
                return if bytes.get(index + 1) == Some(&b'\n') {
                    "\r\n"
                } else {
                    "\r"
                };
            }
            b'\n' => return "\n",
            _ => {}
        }
    }
    "\r\n"
}

fn split_lines_with_ends(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' => {
                let end = if bytes.get(index + 1) == Some(&b'\n') {
                    index + 2
                } else {
                    index + 1
                };
                lines.push(&text[start..end]);
                index = end;
                start = end;
            }
            b'\n' => {
                lines.push(&text[start..index + 1]);
                index += 1;
                start = index;
            }
            _ => index += 1,
        }
    }
    if start < bytes.len() {
        lines.push(&text[start..]);
    }
    lines
}

fn temporary_sibling(path: &Path) -> std::path::PathBuf {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    directory.join(format!("dlssg-config-{stamp}.tmp"))
}

/// 按原编码写回，BOM 原样保留。
fn encode(decoded: &DecodedText, text: &str) -> Result<Vec<u8>> {
    let mut out = decoded.bom.clone();
    match decoded.encoding {
        TextEncoding::Utf8 | TextEncoding::Utf8Bom => out.extend_from_slice(text.as_bytes()),
        TextEncoding::Utf16Le => {
            for unit in text.encode_utf16() {
                out.extend_from_slice(&unit.to_le_bytes());
            }
        }
        TextEncoding::Utf16Be => {
            for unit in text.encode_utf16() {
                out.extend_from_slice(&unit.to_be_bytes());
            }
        }
        TextEncoding::Utf32Le => {
            for character in text.chars() {
                out.extend_from_slice(&(character as u32).to_le_bytes());
            }
        }
        TextEncoding::Utf32Be => {
            for character in text.chars() {
                out.extend_from_slice(&(character as u32).to_be_bytes());
            }
        }
        TextEncoding::Gbk => {
            let (encoded, _, had_errors) = encoding_rs::GBK.encode(text);
            if had_errors {
                return Err(Error::invalid(
                    "写回内容含 GBK 无法表示的字符，已停止写回。".to_owned(),
                ));
            }
            out.extend_from_slice(&encoded);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_recorded_encoding_decodes_to_the_same_text() {
        let index: serde_json::Value =
            serde_json::from_str(&crate::fixtures::read_text("config/encodings/index.json"))
                .expect("索引应可解析");
        let expected = index["Text"].as_str().expect("需要 Text");
        for relative in index["Files"].as_array().expect("需要 Files") {
            let relative = relative.as_str().expect("文件名");
            let name = relative.rsplit('/').next().expect("文件名");
            let kind = name
                .trim_start_matches("启动配置-")
                .trim_end_matches(".ini");
            let expected_encoding = match kind {
                "utf8" => TextEncoding::Utf8,
                "utf8-bom" => TextEncoding::Utf8Bom,
                "utf16-le" => TextEncoding::Utf16Le,
                "utf16-be" => TextEncoding::Utf16Be,
                "utf32-le" => TextEncoding::Utf32Le,
                "utf32-be" => TextEncoding::Utf32Be,
                "gbk" => TextEncoding::Gbk,
                other => panic!("未知编码样本：{other}"),
            };
            let bytes = crate::fixtures::read_bytes(relative);
            let decoded = decode(&bytes).unwrap_or_else(|error| panic!("{relative}：{error}"));
            assert_eq!(decoded.text, expected, "编码样本不一致：{relative}");
            assert_eq!(decoded.encoding, expected_encoding, "{relative} 识别错误");
            if expected_encoding == TextEncoding::Gbk {
                assert_eq!(
                    decoded.warnings.len(),
                    1,
                    "GBK 回退应给出一条告警，实际：{:?}",
                    decoded.warnings
                );
            } else {
                assert!(decoded.warnings.is_empty(), "{relative} 不应有告警");
            }
        }
    }

    #[test]
    fn bom_bytes_are_preserved_for_writeback() {
        let with_bom = decode(&crate::fixtures::read_bytes(
            "config/encodings/启动配置-utf8-bom.ini",
        ))
        .expect("解码");
        assert_eq!(with_bom.bom, vec![0xEF, 0xBB, 0xBF]);
        assert!(with_bom.writes_bom());

        let plain = decode(&crate::fixtures::read_bytes(
            "config/encodings/启动配置-utf8.ini",
        ))
        .expect("解码");
        assert!(plain.bom.is_empty());
        assert!(!plain.writes_bom());
    }

    #[test]
    fn utf32_is_detected_before_utf16() {
        let le = decode(&crate::fixtures::read_bytes(
            "config/encodings/启动配置-utf32-le.ini",
        ))
        .expect("解码");
        assert_eq!(le.encoding, TextEncoding::Utf32Le);
        let be = decode(&crate::fixtures::read_bytes(
            "config/encodings/启动配置-utf32-be.ini",
        ))
        .expect("解码");
        assert_eq!(be.encoding, TextEncoding::Utf32Be);
    }

    #[test]
    fn parsed_sample_keeps_chinese_keys_aliases_and_inline_characters() {
        let text = crate::fixtures::read_text("config/encodings/启动配置-utf8.ini");
        let document = parse(&text).expect("解析");

        let wuthering = document.section("鸣潮").expect("应有鸣潮分组");
        assert_eq!(wuthering.label, "Wuthering");
        assert_eq!(wuthering.line, 3);
        assert_eq!(
            wuthering
                .get("GameDirectory")
                .map(|entry| entry.value.as_str()),
            Some(r"D:\Games\WuWa")
        );
        // 行内注释不切分：`#` 与 `=` 都保留在值里
        assert_eq!(
            wuthering
                .get("LauncherPath")
                .map(|entry| entry.value.as_str()),
            Some(r"C:\a#b=c\launcher.exe")
        );
        assert_eq!(
            wuthering
                .get("RestartGraceSeconds")
                .map(|entry| entry.value.as_str()),
            Some("10")
        );

        let gpu = document.section("显卡").expect("显卡别名应可查到 Gpu");
        assert_eq!(gpu.label, "Gpu");
        assert_eq!(
            gpu.get("DeviceInstanceId")
                .map(|entry| entry.value.as_str()),
            Some(r"HDAUDIO\FUNC_01")
        );
    }

    #[test]
    fn line_numbers_point_at_the_offending_line() {
        let error = parse("[鸣潮]\n游戏目录=D:\\x\n坏的键\n").expect_err("应报错");
        let text = error.to_string();
        assert!(text.contains("第 3 行"), "实际：{text}");
    }

    #[test]
    fn key_value_before_any_section_is_rejected() {
        let error = parse("游戏目录=D:\\x\n").expect_err("应报错");
        assert!(error.to_string().contains("第 1 行"));
    }

    #[test]
    fn duplicate_section_and_duplicate_key_are_rejected() {
        let duplicate_section = parse("[鸣潮]\n[Wuthering]\n").expect_err("分组别名重复");
        assert!(
            duplicate_section.to_string().contains("重复"),
            "实际：{duplicate_section}"
        );

        let duplicate_key = parse("[鸣潮]\n游戏目录=a\nGameDirectory=b\n").expect_err("键别名重复");
        assert!(
            duplicate_key.to_string().contains("重复"),
            "实际：{duplicate_key}"
        );
    }

    #[test]
    fn malformed_header_is_a_format_error() {
        let error = parse("[鸣潮\n游戏目录=a\n").expect_err("应报错");
        assert!(error.to_string().contains("第 1 行"), "实际：{error}");
    }

    #[test]
    fn all_three_line_endings_are_accepted() {
        let document = parse("[鸣潮]\r\n游戏目录=a\n配置源=b\r重启等待秒数=3").expect("解析");
        let section = document.section("Wuthering").expect("分组");
        assert_eq!(section.entries.len(), 3);
        assert_eq!(section.entries[2].value, "3");
    }

    #[test]
    fn unquote_handles_ascii_and_cjk_quotes() {
        assert_eq!(unquote("\"a b\"", "x").expect("去引号"), "a b");
        assert_eq!(unquote("'a'", "x").expect("去引号"), "a");
        assert_eq!(unquote("\u{201C}a\u{201D}", "x").expect("去引号"), "a");
        assert_eq!(unquote("plain", "x").expect("去引号"), "plain");
        assert!(unquote("\"open", "x").is_err());
    }

    #[test]
    fn nul_bytes_are_rejected() {
        assert!(parse("[鸣潮]\0\n").is_err());
    }
}
