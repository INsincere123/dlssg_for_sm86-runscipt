//! PE 导入表扫描与代理推荐（M3-04）。
//!
//! 目的：**只推荐游戏真正引用的代理名**。UE 游戏不一定导入 `d3d12.dll`
//! （可能走 `dxgi.dll` 或静态链 D3D12），盲目用默认 `version.dll` 可能根本不生效。
//!
//! 支持 PE32 与 PE32+，只看导入目录（DataDirectory[1]），不认识绑定导入的细节，
//! 但能读出 DLL 名（`OriginalFirstThunk` 为 0 也不影响 `Name`）。
//! 解析失败一律当作"没有导入信息"，由调用方回退默认代理，不抛异常去阻断启动。

use std::path::Path;

use crate::error::{Error, Result};
use crate::resources::PROXY_NAMES;

/// 一次导入解析的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// 按导入表顺序、去重后的 DLL 名（原样大小写）。
    pub libraries: Vec<String>,
}

impl ImportReport {
    /// 与已知代理名求交集（大小写不敏感），保持 `PROXY_NAMES` 的顺序。
    pub fn proxy_candidates(&self) -> Vec<String> {
        PROXY_NAMES
            .iter()
            .filter(|name| {
                self.libraries
                    .iter()
                    .any(|library| library.eq_ignore_ascii_case(name))
            })
            .map(|name| (*name).to_owned())
            .collect()
    }
}

/// 读取 PE 的导入 DLL 列表。
pub fn imported_libraries(path: &Path) -> Result<ImportReport> {
    let data = std::fs::read(path).map_err(|error| Error::io(path, error))?;
    parse_imports(&data)
        .ok_or_else(|| Error::invalid(format!("不是有效的 PE 文件：{}", path.display())))
}

/// 从字节解析；任何结构不合规都返回 `None`（调用方视为"无导入信息"）。
pub fn parse_imports(data: &[u8]) -> Option<ImportReport> {
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
    if directories < 2 {
        return None;
    }
    let import_rva = read_u32(data, directory_base + 8)? as usize;
    if import_rva == 0 {
        return Some(ImportReport::default());
    }

    let table = optional + optional_size;
    if table + sections * 40 > data.len() {
        return None;
    }
    let section_count = sections.min((data.len() - table) / 40);
    let mut map: Vec<(usize, usize, usize, usize)> = Vec::new();
    for index in 0..section_count {
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

    let mut libraries: Vec<String> = Vec::new();
    let mut descriptor = import_rva;
    let mut guard = 0usize;
    loop {
        // 描述符 20 字节；全零即结束。
        if guard > 4096 {
            return None;
        }
        guard += 1;
        let start = rva_to_offset(&map, descriptor, data.len())?;
        if start + 20 > data.len() {
            return None;
        }
        let descriptor_bytes = &data[start..start + 20];
        if descriptor_bytes.iter().all(|byte| *byte == 0) {
            break;
        }
        let name_rva = read_u32(data, start + 12)? as usize;
        let name_offset = rva_to_offset(&map, name_rva, data.len())?;
        let name = read_c_string(data, name_offset)?;
        if !libraries
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&name))
        {
            libraries.push(name);
        }
        descriptor += 20;
    }
    Some(ImportReport { libraries })
}

fn rva_to_offset(
    map: &[(usize, usize, usize, usize)],
    rva: usize,
    file_length: usize,
) -> Option<usize> {
    for (virtual_address, span, raw_size, raw_pointer) in map {
        if rva >= *virtual_address && rva < virtual_address + span {
            let delta = rva - virtual_address;
            if delta >= *raw_size {
                return None;
            }
            let offset = raw_pointer + delta;
            return (offset < file_length).then_some(offset);
        }
    }
    None
}

fn read_c_string(data: &[u8], offset: usize) -> Option<String> {
    let end = data[offset..].iter().position(|byte| *byte == 0)?;
    let text = std::str::from_utf8(&data[offset..offset + end]).ok()?;
    (!text.is_empty()).then(|| text.to_owned())
}

fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    let bytes = data.get(offset..offset + 2)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    let bytes = data.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一个最小 x64 PE：一个 `.idata` 段（RVA 0x1000 / 文件偏移 0x200）。
    fn sample_pe(names: &[&str], pe32: bool) -> Vec<u8> {
        let mut data = vec![0u8; 0x400];
        data[0..2].copy_from_slice(b"MZ");
        let nt = 0x80usize;
        data[60..64].copy_from_slice(&(nt as u32).to_le_bytes());
        data[nt..nt + 4].copy_from_slice(b"PE\0\0");
        let machine: u16 = if pe32 { 0x014c } else { 0x8664 };
        data[nt + 4..nt + 6].copy_from_slice(&machine.to_le_bytes());
        data[nt + 6..nt + 8].copy_from_slice(&1u16.to_le_bytes()); // sections
        let optional_size: usize = if pe32 { 0xE0 } else { 0xF0 };
        data[nt + 20..nt + 22].copy_from_slice(&(optional_size as u16).to_le_bytes());
        let optional = nt + 24;
        let magic: u16 = if pe32 { 0x10b } else { 0x20b };
        data[optional..optional + 2].copy_from_slice(&magic.to_le_bytes());
        let (directory_base, count_offset) = if pe32 {
            (optional + 96, optional + 92)
        } else {
            (optional + 112, optional + 108)
        };
        data[count_offset..count_offset + 4].copy_from_slice(&16u32.to_le_bytes());
        // 导入目录：RVA 0x1000，大小 0x40
        data[directory_base + 8..directory_base + 12].copy_from_slice(&0x1000u32.to_le_bytes());
        data[directory_base + 12..directory_base + 16].copy_from_slice(&0x40u32.to_le_bytes());
        // 段表
        let table = optional + optional_size;
        data[table..table + 8].copy_from_slice(b".idata\0\0");
        data[table + 8..table + 12].copy_from_slice(&0x400u32.to_le_bytes()); // VirtualSize
        data[table + 12..table + 16].copy_from_slice(&0x1000u32.to_le_bytes()); // VirtualAddress
        data[table + 16..table + 20].copy_from_slice(&0x200u32.to_le_bytes()); // SizeOfRawData
        data[table + 20..table + 24].copy_from_slice(&0x200u32.to_le_bytes()); // PointerToRawData

        // 段内容：N 个描述符 + 名字
        let base = 0x200usize;
        let mut name_offset = base + 20 * (names.len() + 1);
        for (index, name) in names.iter().enumerate() {
            let descriptor = base + index * 20;
            let rva = 0x1000 + (name_offset - base);
            data[descriptor + 12..descriptor + 16].copy_from_slice(&(rva as u32).to_le_bytes());
            data[name_offset..name_offset + name.len()].copy_from_slice(name.as_bytes());
            name_offset += name.len() + 1;
        }
        data
    }

    #[test]
    fn parses_imports_and_offers_known_proxies() {
        let data = sample_pe(&["KERNEL32.dll", "D3D12.dll", "version.dll"], false);
        let report = parse_imports(&data).expect("解析");
        assert_eq!(
            report.libraries,
            vec!["KERNEL32.dll", "D3D12.dll", "version.dll"]
        );
        assert_eq!(
            report.proxy_candidates(),
            vec!["version.dll".to_owned(), "d3d12.dll".to_owned()],
            "候选按 PROXY_NAMES 顺序且大小写不敏感"
        );
    }

    #[test]
    fn pe32_and_pe32_plus_are_both_supported() {
        for pe32 in [true, false] {
            let data = sample_pe(&["dxgi.dll"], pe32);
            let report = parse_imports(&data).expect("解析");
            assert_eq!(report.proxy_candidates(), vec!["dxgi.dll".to_owned()]);
        }
    }

    #[test]
    fn no_import_directory_means_no_candidates() {
        let mut data = sample_pe(&["dxgi.dll"], false);
        let nt = 0x80usize;
        let directory_base = nt + 24 + 112;
        data[directory_base + 8..directory_base + 12].copy_from_slice(&0u32.to_le_bytes());
        let report = parse_imports(&data).expect("解析");
        assert!(report.libraries.is_empty());
        assert!(report.proxy_candidates().is_empty());
    }

    #[test]
    fn garbage_is_not_a_pe() {
        assert!(parse_imports(b"not a pe at all").is_none());
        assert!(parse_imports(&[]).is_none());
        let mut zero = vec![0u8; 256];
        zero[0] = b'M';
        zero[1] = b'Z';
        assert!(parse_imports(&zero).is_none());
    }

    #[test]
    fn missing_file_is_an_io_error() {
        let path = std::env::temp_dir().join("dlssg-pe-not-here.exe");
        let error = imported_libraries(&path).expect_err("应报错");
        assert!(matches!(error, Error::Io { .. }), "{error:?}");
    }
}
