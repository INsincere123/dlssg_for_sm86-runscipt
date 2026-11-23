//! UE4 / UE5 主版本识别。
//!
//! 口径与 `启动脚本/scripts/common/UnrealQuality.ps1` 一致，**证据冲突或缺失时不猜**：
//!
//! 1. `<游戏>\Engine\Build\Build.version` 里的 `"MajorVersion": 4|5`
//! 2. 渲染 EXE 里内嵌的标记 `++UE<4|5>+Release-<4|5>.<次版本>`（ASCII 与 UTF-16LE 都扫，
//!    分块读取并保留 256 字节重叠，跨块边界的标记也能命中）
//! 3. 渲染 EXE 的版本信息资源（`RT_VERSION`）：
//!    - `ProductVersion` / `FileVersion` 里的 `++UE5+Release-5.x` 或 `Unreal Engine 5.x`
//!    - `ProductName` / `FileDescription` 含 `Unreal Engine` 时，`ProductVersion` / `FileVersion`
//!      以 `4.` / `5.` 开头也算
//!
//! 同一主版本的多条证据只算一票；出现两个主版本就是"冲突"，返回 `None` 让界面要求手动校准。
//! 识别成功也只代表主版本，不代表 CVar 或组件兼容（与 PS 的说明一致）。
//!
//! `.utoc` / `.ucas` 之类的文件**不作为** UE5 的证据。

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// 扫描 EXE 标记时的分块大小。
const CHUNK: usize = 1024 * 1024;
/// 跨块保留的重叠字节数；小于最长标记（`++UE5+Release-5.5.0` 约 20 字节）即可。
const OVERLAP: usize = 256;
/// 最多扫描 EXE 的多少字节（再大就只扫前 256 MiB，并在证据里说明）。
const MAX_SCAN_BYTES: u64 = 256 * 1024 * 1024;
/// `Build.version` 的大小上限（超过就当作不可信，不解析）。
const MAX_BUILD_VERSION_BYTES: u64 = 1024 * 1024;

/// 一次识别的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Detection {
    /// 唯一主版本；`None` 表示未确定或证据冲突。
    pub major: Option<u8>,
    /// 两个主版本都有证据。
    pub conflict: bool,
    /// 结论来源（界面直接显示）。
    pub source: String,
    /// 逐条证据，形如 `5：EXE 标记 ++UE5+Release-5.4.2`。
    pub evidence: Vec<String>,
}

impl Detection {
    /// 是否已经确定主版本。
    pub fn is_known(&self) -> bool {
        self.major.is_some()
    }

    /// 界面文案：`UE5` / `UE4` / `UE4/UE5识别证据冲突，请手动校准` / `无法确定`。
    pub fn text(&self) -> String {
        match (self.major, self.conflict) {
            (Some(4), _) => "UE4".to_owned(),
            (Some(5), _) => "UE5".to_owned(),
            (None, true) => "UE4/UE5识别证据冲突，请手动校准".to_owned(),
            (None, false) => "无法确定".to_owned(),
            (Some(_), _) => "无法确定".to_owned(),
        }
    }
}

/// 识别一个游戏的引擎主版本。
pub fn detect(game_directory: &Path, executable: &Path) -> Detection {
    let mut evidence: Vec<(u8, String)> = Vec::new();
    if let Some(item) = from_build_version(game_directory) {
        evidence.push(item);
    }
    match scan_markers(executable) {
        Ok(items) => evidence.extend(items),
        Err(error) => evidence.push((0, format!("读取 EXE 失败：{error}"))),
    }
    evidence.extend(from_version_info(executable));
    collect(evidence)
}

/// 把多条证据折算成结论。
fn collect(evidence: Vec<(u8, String)>) -> Detection {
    let mut majors: Vec<u8> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    for (major, line) in evidence {
        if major == 4 || major == 5 {
            if !majors.contains(&major) {
                majors.push(major);
            }
            lines.push(format!("{major}：{line}"));
        } else if !line.is_empty() {
            // 非证据（例如读取失败）只进说明，不参与投票。
            lines.push(line);
        }
    }
    let conflict = majors.len() > 1;
    let major = if majors.len() == 1 {
        Some(majors[0])
    } else {
        None
    };
    let source = if conflict {
        "UE4/UE5识别证据冲突，请手动校准".to_owned()
    } else if let Some(major) = major {
        let reasons: Vec<&str> = lines
            .iter()
            .filter(|line| line.starts_with(&format!("{major}：")))
            .map(|line| {
                line.split_once("：")
                    .map(|(_, rest)| rest)
                    .unwrap_or(line.as_str())
            })
            .collect();
        reasons.join("；")
    } else {
        "无法确定".to_owned()
    };
    Detection {
        major,
        conflict,
        source,
        evidence: lines,
    }
}

/// `Engine\Build\Build.version` 里的 `MajorVersion`。
fn from_build_version(game_directory: &Path) -> Option<(u8, String)> {
    let path: PathBuf = game_directory
        .join("Engine")
        .join("Build")
        .join("Build.version");
    let metadata = std::fs::metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_BUILD_VERSION_BYTES {
        return None;
    }
    let text = std::fs::read_to_string(&path).ok()?;
    let major = json_number(&text, "MajorVersion")?;
    let major = u8::try_from(major).ok()?;
    (major == 4 || major == 5).then(|| (major, "Build.version".to_owned()))
}

/// 从 JSON 文本里取一个整数键（只认 `"键"` 后第一个 `:` 的十进制数字）。
fn json_number(text: &str, key: &str) -> Option<u64> {
    let needle = format!("\"{key}\"");
    let start = text.find(&needle)? + needle.len();
    let rest = &text[start..];
    let colon = rest.find(':')?;
    let digits: String = rest[colon + 1..]
        .trim_start()
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect();
    (!digits.is_empty()).then(|| digits.parse().ok()).flatten()
}

/// 扫 EXE 里的 `++UE<major>+Release-<major>.<minor>` 标记。
pub fn scan_markers(executable: &Path) -> Result<Vec<(u8, String)>> {
    scan_markers_with_chunk(executable, CHUNK)
}

/// 分块大小可注入的版本（测试跨块边界用）。
pub fn scan_markers_with_chunk(executable: &Path, chunk: usize) -> Result<Vec<(u8, String)>> {
    use std::io::Read;

    let chunk = chunk.max(OVERLAP * 2);
    let mut file = std::fs::File::open(executable).map_err(|error| Error::io(executable, error))?;
    let mut buffer = vec![0u8; chunk + OVERLAP];
    let mut tail = 0usize;
    let mut scanned: u64 = 0;
    let mut found: Vec<(u8, String)> = Vec::new();
    loop {
        let read = file
            .read(&mut buffer[tail..tail + chunk])
            .map_err(|error| Error::io(executable, error))?;
        if read == 0 {
            break;
        }
        scanned += read as u64;
        let count = tail + read;
        // ASCII
        collect_markers(&buffer[..count], false, &mut found);
        // UTF-16LE：偶数对齐与奇数对齐各扫一次，避免漏掉错位存储的字符串
        let even = count - count % 2;
        collect_markers(&buffer[..even], true, &mut found);
        if count > 1 {
            let odd = (count - 1) - (count - 1) % 2;
            collect_markers(&buffer[1..1 + odd], true, &mut found);
        }
        if scanned >= MAX_SCAN_BYTES {
            break;
        }
        tail = OVERLAP.min(count);
        buffer.copy_within(count - tail..count, 0);
    }
    Ok(found)
}

/// 在字节里找 `++UE` 标记；`utf16` 为真时按 UTF-16LE 解释。
fn collect_markers(data: &[u8], utf16: bool, found: &mut Vec<(u8, String)>) {
    let step = if utf16 { 2 } else { 1 };
    let width = |text: &str| {
        if utf16 { text.len() * 2 } else { text.len() }
    };
    let bytes_at = |offset: usize, text: &str| -> Option<()> {
        for (index, character) in text.bytes().enumerate() {
            let position = offset + index * step;
            if position >= data.len() || data[position] != character {
                return None;
            }
        }
        Some(())
    };
    let read_digit = |offset: usize| -> Option<u8> {
        let position = offset;
        if position >= data.len() {
            return None;
        }
        let byte = data[position];
        if utf16 && (position + 1 >= data.len() || data[position + 1] != 0) {
            return None;
        }
        (byte.is_ascii_digit()).then_some(byte - b'0')
    };

    let mut offset = 0usize;
    while offset + width("++UE") <= data.len() {
        if bytes_at(offset, "++UE").is_none() {
            offset += step;
            continue;
        }
        let digit_at = offset + width("++UE");
        let Some(major) = read_digit(digit_at) else {
            offset += step;
            continue;
        };
        let after_major = digit_at + step;
        if bytes_at(after_major, "+Release-").is_none() {
            offset += step;
            continue;
        }
        let release_at = after_major + width("+Release-");
        let Some(release) = read_digit(release_at) else {
            offset += step;
            continue;
        };
        // 次版本必须跟着 `.`；`++UE5+Release-5.4.2` 这种才认。
        let after_release = release_at + step;
        let dot_ok = after_release < data.len() && data[after_release] == b'.';
        if (major == 4 || major == 5) && major == release && dot_ok {
            let text = format!("EXE 标记 ++UE{major}+Release-{release}");
            if !found.iter().any(|(existing, _)| *existing == major) {
                found.push((major, text));
            }
        }
        offset += step;
    }
}

/// 从 PE 的版本信息资源里读 `ProductVersion` 等字符串。
fn from_version_info(executable: &Path) -> Vec<(u8, String)> {
    let Some(strings) = version_strings(executable) else {
        return Vec::new();
    };
    let get = |key: &str| -> Option<&str> {
        strings
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.as_str())
    };
    let mut found: Vec<(u8, String)> = Vec::new();
    let mut add = |major: u8, line: String| {
        if major == 4 || major == 5 && !found.iter().any(|(existing, _)| *existing == major) {
            found.push((major, line));
        }
    };
    let product = get("ProductName").unwrap_or_default();
    let description = get("FileDescription").unwrap_or_default();
    let names_unreal = contains_ignore_case(product, "Unreal Engine")
        || contains_ignore_case(description, "Unreal Engine");
    for key in [
        "ProductVersion",
        "FileVersion",
        "ProductName",
        "FileDescription",
    ] {
        let Some(value) = get(key) else { continue };
        if let Some(major) = marker_major(value) {
            add(major, format!("EXE 版本信息 {key} {value}"));
        }
        if let Some(major) = unreal_engine_major(value) {
            add(major, format!("EXE 版本信息 {key} {value}"));
        }
        // `UE5-CL-0` 这种显式前缀（异环的 ProductVersion 就是这样）。
        // 比 PS 多认这一条：字符串本身就写着主版本，不算猜测。
        if let Some(major) = ue_prefix_major(value) {
            add(major, format!("EXE 版本信息 {key} {value}"));
        }
    }
    if names_unreal {
        for key in ["ProductVersion", "FileVersion"] {
            let Some(value) = get(key) else { continue };
            if let Some(major) = leading_major(value) {
                add(major, format!("EXE 版本信息 {key} {value}"));
            }
        }
    }
    found
}

/// 读 PE 里的版本信息资源并解析成键值对；读不到返回 `None`。
pub fn version_strings(executable: &Path) -> Option<Vec<(String, String)>> {
    let data = std::fs::read(executable).ok()?;
    let blob = find_version_blob(&data)?;
    Some(parse_version_info(blob))
}

/// 读 PE 的 `.rsrc`，按 `RT_VERSION`(16) → 第一个名字 → 第一个语言 定位数据块。
fn find_version_blob(data: &[u8]) -> Option<&[u8]> {
    if data.len() < 64 || &data[0..2] != b"MZ" {
        return None;
    }
    let nt = read_u32(data, 60)? as usize;
    if nt + 24 > data.len() || &data[nt..nt + 4] != b"PE\0\0" {
        return None;
    }
    let sections = read_u16(data, nt + 6)? as usize;
    let optional_size = read_u16(data, nt + 20)? as usize;
    let optional = nt + 24;
    let magic = read_u16(data, optional)?;
    let (directory_base, count_offset) = match magic {
        0x10b => (optional + 96, optional + 92),
        0x20b => (optional + 112, optional + 108),
        _ => return None,
    };
    let directories = read_u32(data, count_offset)? as usize;
    if directories < 3 {
        return None;
    }
    // DataDirectory[2] = 资源表
    let resource_rva = read_u32(data, directory_base + 16)? as usize;
    if resource_rva == 0 {
        return None;
    }
    let table = optional + optional_size;
    if table + sections * 40 > data.len() {
        return None;
    }
    let mut map: Vec<(usize, usize, usize, usize)> = Vec::new();
    for index in 0..sections {
        let base = table + index * 40;
        let virtual_size = read_u32(data, base + 8)? as usize;
        let virtual_address = read_u32(data, base + 12)? as usize;
        let raw_size = read_u32(data, base + 16)? as usize;
        let raw_pointer = read_u32(data, base + 20)? as usize;
        map.push((
            virtual_address,
            virtual_size.max(raw_size),
            raw_size,
            raw_pointer,
        ));
    }
    let rva_to_offset = |rva: usize| -> Option<usize> {
        for (virtual_address, span, raw_size, raw_pointer) in &map {
            if rva >= *virtual_address && rva < virtual_address + span {
                let delta = rva - virtual_address;
                if delta >= *raw_size {
                    return None;
                }
                let offset = raw_pointer + delta;
                return (offset < data.len()).then_some(offset);
            }
        }
        None
    };

    // 资源目录：16 字节头（含 named/id 数量）+ 8 字节一项。
    // 每一层的偏移都是**相对资源根**的，不是相对当前目录（这里曾经写错，真机上一直取不到）。
    const HEADER: usize = 16;
    let root = rva_to_offset(resource_rva)?;
    let entry_count = |directory: usize| -> Option<usize> {
        let named = read_u16(data, directory + 12)? as usize;
        let ids = read_u16(data, directory + 14)? as usize;
        Some(named + ids)
    };
    let entry = |directory: usize, index: usize| -> Option<(u32, usize)> {
        let at = directory + HEADER + index * 8;
        let id = read_u32(data, at)?;
        let offset = (read_u32(data, at + 4)? & 0x7fff_ffff) as usize;
        Some((id, offset))
    };

    // 第一层：类型，找 RT_VERSION = 16
    let count = entry_count(root)?;
    let mut type_offset = None;
    for index in 0..count {
        let (id, offset) = entry(root, index)?;
        if id == 16 {
            type_offset = Some(offset);
            break;
        }
    }
    // 第二层：名字（取第一个）
    let name_directory = root + type_offset?;
    let (_, name_offset) = entry(name_directory, 0)?;
    // 第三层：语言（取第一个）
    let language_directory = root + name_offset;
    let (_, data_offset) = entry(language_directory, 0)?;
    // 第四层：数据项（也相对资源根）：RVA(4) + Size(4) + CodePage(4) + Reserved(4)
    let data_entry = root + data_offset;
    let blob_rva = read_u32(data, data_entry)? as usize;
    let blob_size = read_u32(data, data_entry + 4)? as usize;
    let blob_offset = rva_to_offset(blob_rva)?;
    (blob_offset + blob_size <= data.len()).then(|| &data[blob_offset..blob_offset + blob_size])
}

/// 解析 `VS_VERSIONINFO`，返回 `StringFileInfo` 里的所有字符串键值。
pub fn parse_version_info(blob: &[u8]) -> Vec<(String, String)> {
    let mut strings = Vec::new();
    // 根块：wLength, wValueLength, wType, szKey("VS_VERSION_INFO"), pad, VS_FIXEDFILEINFO, 子块。
    // 注意 wLength 是整个结构（含子块）的长度，**不能**用它的结尾当子块起点；
    // 子块起点要跳过键名和 wValueLength 个字节的 VS_FIXEDFILEINFO。
    let Some(value_length) = read_u16(blob, 2).map(|value| value as usize) else {
        return strings;
    };
    let Some(key) = block_key(blob, 0) else {
        return strings;
    };
    let value_start = align4(6 + (key.chars().count() + 1) * 2);
    let mut offset = align4(value_start + value_length);
    // 之后可能先有 VS_FIXEDFILEINFO（wValueLength 个字节），再是子块。
    // 直接按块遍历：每个子块 wLength 自描述，遇到 0 结束。
    while offset + 6 <= blob.len() {
        let Some(end) = block_end(blob, offset) else {
            break;
        };
        if end <= offset {
            break;
        }
        let key = block_key(blob, offset).unwrap_or_default();
        if key.eq_ignore_ascii_case("StringFileInfo") {
            collect_string_tables(blob, offset, end, &mut strings);
            break;
        }
        offset = align4(end);
    }
    strings
}

/// 在 `StringFileInfo` 块里逐个子表收集字符串。
fn collect_string_tables(blob: &[u8], start: usize, end: usize, out: &mut Vec<(String, String)>) {
    let mut offset = align4(start + header_len(blob, start).unwrap_or(6));
    while offset + 6 <= end {
        let Some(table_end) = block_end(blob, offset).filter(|value| *value <= end) else {
            break;
        };
        if table_end <= offset {
            break;
        }
        let mut entry = align4(offset + header_len(blob, offset).unwrap_or(6));
        while entry + 6 <= table_end {
            let Some(entry_end) = block_end(blob, entry).filter(|value| *value <= table_end) else {
                break;
            };
            if entry_end <= entry {
                break;
            }
            if let Some((name, value)) = string_entry(blob, entry) {
                out.push((name, value));
            }
            entry = align4(entry_end);
        }
        offset = table_end;
    }
}

/// 一个字符串块 = `wLength, wValueLength, wType, szKey\0, pad, szValue\0`。
fn string_entry(blob: &[u8], start: usize) -> Option<(String, String)> {
    let value_length = read_u16(blob, start + 2)? as usize;
    let key = block_key(blob, start)?;
    let key_bytes = (key.chars().count() + 1) * 2;
    let value_start = align4(start + 6 + key_bytes);
    let value = read_utf16z(blob, value_start, value_length)?;
    Some((key, value))
}

/// 块头之后的键名（UTF-16，空串表示没有）。
fn block_key(blob: &[u8], start: usize) -> Option<String> {
    let key_start = start + 6;
    let mut units: Vec<u16> = Vec::new();
    let mut offset = key_start;
    while offset + 2 <= blob.len() {
        let unit = u16::from_le_bytes([blob[offset], blob[offset + 1]]);
        if unit == 0 {
            break;
        }
        units.push(unit);
        offset += 2;
        if units.len() > 256 {
            return None;
        }
    }
    Some(String::from_utf16_lossy(&units))
}

/// 块的结束偏移 = 起点 + wLength（已经 4 字节对齐）。
fn block_end(blob: &[u8], start: usize) -> Option<usize> {
    let length = read_u16(blob, start)? as usize;
    (length >= 6 && start + length <= blob.len()).then_some(start + length)
}

/// 块头长度 = 6 + 键名（含结尾 0）的字节数。
fn header_len(blob: &[u8], start: usize) -> Option<usize> {
    let key = block_key(blob, start)?;
    Some(6 + (key.chars().count() + 1) * 2)
}

fn align4(value: usize) -> usize {
    (value + 3) & !3
}

fn read_utf16z(blob: &[u8], start: usize, max_units: usize) -> Option<String> {
    let limit = if max_units == 0 {
        usize::MAX
    } else {
        max_units.saturating_sub(1)
    };
    let mut units: Vec<u16> = Vec::new();
    let mut offset = start;
    while offset + 2 <= blob.len() && units.len() <= limit.min(512) {
        let unit = u16::from_le_bytes([blob[offset], blob[offset + 1]]);
        if unit == 0 {
            break;
        }
        units.push(unit);
        offset += 2;
    }
    Some(String::from_utf16_lossy(&units))
}

fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    let bytes = data.get(offset..offset + 2)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    let bytes = data.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// 从一段文本里取 `++UE<major>+Release-<major>.` 的主版本。
fn marker_major(text: &str) -> Option<u8> {
    let bytes = text.as_bytes();
    let needle = b"++UE";
    let mut offset = 0usize;
    while offset + needle.len() < bytes.len() {
        if &bytes[offset..offset + needle.len()] == needle {
            let major = bytes[offset + 4];
            let rest = &bytes[offset + 5..];
            let mut index = 0usize;
            let release_prefix = b"+Release-";
            let matches_prefix = rest.len() >= release_prefix.len()
                && rest[..release_prefix.len()].eq_ignore_ascii_case(release_prefix);
            if matches_prefix && (major == b'4' || major == b'5') {
                index += release_prefix.len();
                if rest.get(index) == Some(&major) && rest.get(index + 1) == Some(&b'.') {
                    return Some(major - b'0');
                }
            }
            let _ = index;
        }
        offset += 1;
    }
    None
}

/// 以 `UE4` / `UE5` 开头（后面不是数字）时返回主版本，例如 `UE5-CL-0`。
fn ue_prefix_major(text: &str) -> Option<u8> {
    let trimmed = text.trim_start();
    let bytes = trimmed.as_bytes();
    if bytes.len() < 3
        || !bytes[0].eq_ignore_ascii_case(&b'U')
        || !bytes[1].eq_ignore_ascii_case(&b'E')
    {
        return None;
    }
    let major = bytes[2];
    if major != b'4' && major != b'5' {
        return None;
    }
    // `UE5x` 这种接着字母的不算（可能是别的缩写）。
    match bytes.get(3) {
        Some(next) if next.is_ascii_alphanumeric() => None,
        _ => Some(major - b'0'),
    }
}

/// 从 `Unreal Engine 5.4` 这类文本里取主版本。
fn unreal_engine_major(text: &str) -> Option<u8> {
    let lower = text.to_ascii_lowercase();
    let position = lower.find("unreal engine")?;
    let rest = lower[position + "unreal engine".len()..].trim_start();
    leading_major(rest)
}

/// 以 `4.` / `5.` 开头时返回主版本。
fn leading_major(text: &str) -> Option<u8> {
    let trimmed = text.trim_start();
    let mut characters = trimmed.chars();
    let first = characters.next()?;
    let second = characters.next()?;
    if (first == '4' || first == '5') && second == '.' {
        return Some(first as u8 - b'0');
    }
    None
}

fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tree(PathBuf);

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tree(name: &str) -> Tree {
        let root = std::env::temp_dir().join(format!("dlssg-ue-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("临时目录");
        Tree(root)
    }

    /// 真机探针：`DLSSG_UE_PROBE=<游戏目录> DLSSG_UE_PROBE_EXE=<渲染 EXE>` 后跑
    /// `cargo test -p launcher-core --lib unreal -- --ignored --nocapture probe_real_game`。
    ///
    /// 默认跳过；只在需要核对真实 PE 的版本信息资源时用。
    #[test]
    #[ignore = "真机检查用：需要 DLSSG_UE_PROBE / DLSSG_UE_PROBE_EXE"]
    fn probe_real_game() {
        let Ok(directory) = std::env::var("DLSSG_UE_PROBE") else {
            return;
        };
        let directory = PathBuf::from(directory);
        let executable = std::env::var("DLSSG_UE_PROBE_EXE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| directory.join("game.exe"));
        println!("版本信息：{:?}", version_strings(&executable));
        println!("结论：{:?}", detect(&directory, &executable));
    }

    #[test]
    fn build_version_json_gives_the_major() {
        let tree = tree("build-version");
        let build = tree.0.join("Engine").join("Build");
        std::fs::create_dir_all(&build).expect("目录");
        std::fs::write(
            build.join("Build.version"),
            b"{\n\t\"MajorVersion\" : 5,\n\t\"MinorVersion\" : 4,\n\t\"PatchVersion\" : 2\n}",
        )
        .expect("写");
        let detection = detect(&tree.0, &tree.0.join("nope.exe"));
        assert_eq!(detection.major, Some(5));
        assert!(detection.source.contains("Build.version"), "{detection:?}");
        assert!(!detection.conflict);
        assert_eq!(detection.text(), "UE5");
    }

    #[test]
    fn markers_are_found_in_ascii_and_utf16_and_across_chunks() {
        let tree = tree("markers");
        let exe = tree.0.join("game.exe");
        let mut data = vec![0u8; 4096];
        // ASCII 标记放在离块边界很近的位置，逼出重叠逻辑
        let marker = b"++UE5+Release-5.4.2";
        let ascii_at = 1024 - 8;
        data[ascii_at..ascii_at + marker.len()].copy_from_slice(marker);
        // UTF-16LE 标记
        let wide: Vec<u8> = "++UE4+Release-4.27"
            .bytes()
            .flat_map(|byte| [byte, 0])
            .collect();
        let wide_at = 2048;
        data[wide_at..wide_at + wide.len()].copy_from_slice(&wide);
        std::fs::write(&exe, &data).expect("写");

        // 故意用很小的块，覆盖"标记跨块"的情况
        let found = scan_markers_with_chunk(&exe, 512).expect("扫描");
        let majors: Vec<u8> = found.iter().map(|(major, _)| *major).collect();
        assert!(majors.contains(&5), "应命中 ASCII 标记：{found:?}");
        assert!(majors.contains(&4), "应命中 UTF-16 标记：{found:?}");
    }

    #[test]
    fn markers_need_the_release_dot_and_matching_digits() {
        let tree = tree("bad-markers");
        let exe = tree.0.join("game.exe");
        // 版本号不匹配、缺 `.`、UE3 都不算证据
        let text = b"++UE5+Release-4.2 ++UE5+Release-5 ++UE3+Release-3.5 ++UE6+Release-6.1";
        std::fs::write(&exe, text).expect("写");
        let found = scan_markers(&exe).expect("扫描");
        assert!(found.is_empty(), "不应有证据：{found:?}");
    }

    #[test]
    fn conflict_is_reported_instead_of_guessing() {
        let tree = tree("conflict");
        let build = tree.0.join("Engine").join("Build");
        std::fs::create_dir_all(&build).expect("目录");
        std::fs::write(build.join("Build.version"), b"{\"MajorVersion\":4}").expect("写");
        let exe = tree.0.join("game.exe");
        std::fs::write(&exe, b"++UE5+Release-5.3.2").expect("写");
        let detection = detect(&tree.0, &exe);
        assert!(detection.conflict);
        assert_eq!(detection.major, None, "冲突时不能给出主版本");
        assert_eq!(detection.text(), "UE4/UE5识别证据冲突，请手动校准");
    }

    #[test]
    fn nothing_found_means_unknown() {
        let tree = tree("unknown");
        let detection = detect(&tree.0, &tree.0.join("missing.exe"));
        assert_eq!(detection.major, None);
        assert!(!detection.conflict);
        assert_eq!(detection.text(), "无法确定");
        assert!(!detection.is_known());
    }

    /// 造一个最小 `VS_VERSIONINFO`：根块 + StringFileInfo + 一张表 + 若干字符串。
    fn version_blob(entries: &[(&str, &str)]) -> Vec<u8> {
        fn utf16z(text: &str) -> Vec<u8> {
            let mut bytes: Vec<u8> = text.bytes().flat_map(|byte| [byte, 0]).collect();
            bytes.extend_from_slice(&[0, 0]);
            bytes
        }
        fn pad4(bytes: &mut Vec<u8>) {
            while bytes.len() % 4 != 0 {
                bytes.push(0);
            }
        }
        fn block(key: &str, value: &[u8], children: &[Vec<u8>], value_length: u16) -> Vec<u8> {
            let mut body = Vec::new();
            body.extend_from_slice(&utf16z(key));
            // 值必须从块的 4 字节边界开始：按**绝对偏移**补 0（键名起点是 6）。
            while (6 + body.len()) % 4 != 0 {
                body.push(0);
            }
            body.extend_from_slice(value);
            for child in children {
                body.extend_from_slice(child);
            }
            let total = 6 + body.len();
            let mut out = Vec::new();
            out.extend_from_slice(&(total as u16).to_le_bytes());
            out.extend_from_slice(&value_length.to_le_bytes());
            out.extend_from_slice(&1u16.to_le_bytes()); // wType = text
            out.extend_from_slice(&body);
            pad4(&mut out);
            out
        }

        let mut strings: Vec<Vec<u8>> = Vec::new();
        for (key, value) in entries {
            let mut bytes = Vec::new();
            let encoded = utf16z(value);
            let mut body = Vec::new();
            body.extend_from_slice(&utf16z(key));
            while (6 + body.len()) % 4 != 0 {
                body.push(0);
            }
            body.extend_from_slice(&encoded);
            let total = 6 + body.len();
            bytes.extend_from_slice(&(total as u16).to_le_bytes());
            bytes.extend_from_slice(&((encoded.len() / 2) as u16).to_le_bytes());
            bytes.extend_from_slice(&1u16.to_le_bytes());
            bytes.extend_from_slice(&body);
            pad4(&mut bytes);
            strings.push(bytes);
        }
        let table = block("080404b0", &[], &strings, 0);
        let string_file_info = block("StringFileInfo", &[], &[table], 0);
        block("VS_VERSION_INFO", &[0u8; 52], &[string_file_info], 52)
    }

    #[test]
    fn version_info_parser_reads_string_file_info() {
        let blob = version_blob(&[
            ("ProductName", "Black Myth: Wukong"),
            ("ProductVersion", "5.3.2.0"),
            ("FileDescription", "Unreal Engine 5.3"),
            ("CompanyName", "Game Science"),
        ]);
        let strings = parse_version_info(&blob);
        let get = |key: &str| {
            strings
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(get("ProductVersion").as_deref(), Some("5.3.2.0"));
        assert_eq!(get("FileDescription").as_deref(), Some("Unreal Engine 5.3"));
        assert_eq!(get("CompanyName").as_deref(), Some("Game Science"));
    }

    #[test]
    fn unreal_engine_text_marks_the_major() {
        // ProductName/FileDescription 含 Unreal Engine + ProductVersion 以 5. 开头
        let blob = version_blob(&[
            ("ProductName", "Unreal Engine"),
            ("ProductVersion", "5.4.1"),
        ]);
        let major = from_blob(&blob);
        assert_eq!(major, Some(5));

        // 直接写 Unreal Engine 4.27
        let blob = version_blob(&[("FileDescription", "Unreal Engine 4.27")]);
        assert_eq!(from_blob(&blob), Some(4));

        // 只有客户名与普通版本号：不能猜
        let blob = version_blob(&[("ProductName", "Some Game"), ("ProductVersion", "1.2.3")]);
        assert_eq!(from_blob(&blob), None);

        // 异环的真实写法：ProductVersion=UE5-CL-0
        let blob = version_blob(&[
            ("ProductName", "异环"),
            ("ProductVersion", "UE5-CL-0"),
            ("FileVersion", "UE5-CL-0"),
        ]);
        assert_eq!(from_blob(&blob), Some(5));
        // 但 UE5x / UE44 这种接着字母数字的不认
        assert_eq!(ue_prefix_major("UE5x"), None);
        assert_eq!(ue_prefix_major("UE44"), None);
        assert_eq!(ue_prefix_major("ue4-cl-1"), Some(4));
    }

    /// 测试辅助：直接把 blob 交给"版本信息 → 主版本"这条链路。
    fn from_blob(blob: &[u8]) -> Option<u8> {
        let strings = parse_version_info(blob);
        let get = |key: &str| -> Option<String> {
            strings
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(key))
                .map(|(_, value)| value.clone())
        };
        let names_unreal = get("ProductName")
            .map(|value| contains_ignore_case(&value, "Unreal Engine"))
            .unwrap_or(false)
            || get("FileDescription")
                .map(|value| contains_ignore_case(&value, "Unreal Engine"))
                .unwrap_or(false);
        for key in [
            "ProductVersion",
            "FileVersion",
            "ProductName",
            "FileDescription",
        ] {
            let Some(value) = get(key) else { continue };
            if let Some(major) = marker_major(&value) {
                return Some(major);
            }
            if let Some(major) = unreal_engine_major(&value) {
                return Some(major);
            }
            if let Some(major) = ue_prefix_major(&value) {
                return Some(major);
            }
        }
        if names_unreal {
            for key in ["ProductVersion", "FileVersion"] {
                let Some(value) = get(key) else { continue };
                if let Some(major) = leading_major(&value) {
                    return Some(major);
                }
            }
        }
        if let Some(value) = get("ProductVersion") {
            if let Some(major) = ue_prefix_major(&value) {
                return Some(major);
            }
        }
        None
    }
}
