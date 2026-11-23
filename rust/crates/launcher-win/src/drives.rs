//! 固定磁盘的根目录（内置游戏浅层查找用）。
//!
//! 只枚举 `DRIVE_FIXED`：网络盘和可移动盘又慢又不会装游戏，扫描不该被它们拖住。
//! 不读卷标、不遍历目录内容——目录遍历在 `launcher_core::builtin::search_game_directory`。

use std::path::PathBuf;

const DRIVE_FIXED: u32 = 3;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetLogicalDrives() -> u32;
    fn GetDriveTypeW(root: *const u16) -> u32;
}

/// 本机每个固定磁盘的根（`C:\` 形式，按盘符升序）；`GetLogicalDrives` 失败时为空。
pub fn fixed_drive_roots() -> Vec<PathBuf> {
    let mask = unsafe { GetLogicalDrives() };
    let mut roots = Vec::new();
    for index in 0..26u32 {
        if mask & (1 << index) == 0 {
            continue;
        }
        let root = format!("{}:\\", (b'A' + index as u8) as char);
        let wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
        if unsafe { GetDriveTypeW(wide.as_ptr()) } == DRIVE_FIXED {
            roots.push(PathBuf::from(root));
        }
    }
    roots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_are_readable_drive_roots() {
        let roots = fixed_drive_roots();
        assert!(!roots.is_empty(), "本机至少有一个固定磁盘：{roots:?}");
        for root in &roots {
            let text = root.to_string_lossy().into_owned();
            assert!(
                text.len() == 3 && text.ends_with(":\\"),
                "根目录形态是 `C:\\`：{text}"
            );
            assert!(root.is_dir(), "固定的盘符必须能访问：{text}");
        }
        let mut sorted = roots.clone();
        sorted.sort();
        assert_eq!(sorted, roots, "按盘符升序返回：{roots:?}");
    }
}
