//! 系统字体的只读内存映射：让 18.8 MB 的微软雅黑不整块进堆。
//!
//! 界面用 `msyh.ttc` 画中文，egui 会把这份字节常驻持有（`epaint::FontData` 里的 `Blob`），
//! 旧做法 `fs::read` 一读就是 18.8 MB 私有内存，占了界面进程的三分之一。epaint 0.37 提供
//! `FontData::from_blob`，注释写明可以包"内存映射的系统字体、不复制"，所以这里只负责映射：
//!
//! 1. 只读映射：`PAGE_READONLY` + `FILE_MAP_READ`，字体文件本来就不该被写；
//! 2. 映射区是**文件页**，不占专用提交，内存吃紧时系统能直接回收；同名字体被别的进程映射
//!    时还能共享物理页；
//! 3. 只有真正被光栅化的字形才会把对应页换入，冷启动也少一次 18.8 MB 的整读；
//! 4. 文件用 `std::fs::File` 打开（共享读写删除，不挡系统更新字体），映射与视图照旧走 Win32；
//! 5. 任何一步失败（文件不在、没权限、空文件、映射失败）都返回 `None`，调用方回退整读。
//!
//! 生命周期：`MappedFont` 同时持有文件与映射句柄，`Drop` 里先解映射再关映射句柄，文件句柄由
//! `File` 自己关。只要外层的 `Blob`（`Arc<MappedFont>`）还活着，视图指针就一直有效。

use std::ffi::c_void;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::sync::Arc;

/// 只读共享的字节，可直接交给 epaint 的 `FontData::from_blob`。
pub type Blob = Arc<dyn AsRef<[u8]> + Send + Sync>;

type Handle = isize;

const INVALID_HANDLE_VALUE: Handle = -1;
const PAGE_READONLY: u32 = 0x0000_0002;
const FILE_MAP_READ: u32 = 0x0000_0004;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateFileMappingW(
        file: Handle,
        attributes: *const c_void,
        protect: u32,
        maximum_high: u32,
        maximum_low: u32,
        name: *const u16,
    ) -> Handle;
    fn MapViewOfFile(
        mapping: Handle,
        access: u32,
        offset_high: u32,
        offset_low: u32,
        bytes: usize,
    ) -> *mut c_void;
    fn UnmapViewOfFile(base: *const c_void) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
}

/// 一个只读映射的文件视图。
pub struct MappedFont {
    base: *const u8,
    length: usize,
    mapping: Handle,
    // 只用来兜住文件句柄：映射本身引用着文件对象，但让 File 活到解映射之后最清楚。
    _file: std::fs::File,
}

// SAFETY: 视图是只读的（`PAGE_READONLY` / `FILE_MAP_READ`），对外只暴露 `&[u8]`；
// 句柄只在 `Drop` 里用，映射区本身与线程无关。
unsafe impl Send for MappedFont {}
unsafe impl Sync for MappedFont {}

impl MappedFont {
    /// 映射到的字节数。
    pub fn len(&self) -> usize {
        self.length
    }

    /// 空映射不会产生（空文件直接回退整读），保留给 `len` 的常规配对。
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// 映射区的只读视图。
    pub fn bytes(&self) -> &[u8] {
        // SAFETY: `base` 指向 `length` 字节的只读映射，`Drop` 之前一直有效。
        unsafe { std::slice::from_raw_parts(self.base, self.length) }
    }
}

impl AsRef<[u8]> for MappedFont {
    fn as_ref(&self) -> &[u8] {
        self.bytes()
    }
}

impl Drop for MappedFont {
    fn drop(&mut self) {
        unsafe {
            if !self.base.is_null() {
                UnmapViewOfFile(self.base.cast());
            }
            if self.mapping != 0 && self.mapping != INVALID_HANDLE_VALUE {
                CloseHandle(self.mapping);
            }
        }
    }
}

/// 把 `path` 只读映射进地址空间；失败返回 `None`（调用方回退整读）。
pub fn map_read_only(path: &Path) -> Option<Arc<MappedFont>> {
    let file = std::fs::File::open(path).ok()?;
    let length = usize::try_from(file.metadata().ok()?.len()).ok()?;
    if length == 0 {
        return None;
    }

    let mapping = unsafe {
        CreateFileMappingW(
            file.as_raw_handle() as Handle,
            std::ptr::null(),
            PAGE_READONLY,
            0,
            0,
            std::ptr::null(),
        )
    };
    if mapping == 0 || mapping == INVALID_HANDLE_VALUE {
        return None;
    }

    let base = unsafe { MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, length) };
    if base.is_null() {
        unsafe { CloseHandle(mapping) };
        return None;
    }

    Some(Arc::new(MappedFont {
        base: base.cast::<u8>().cast_const(),
        length,
        mapping,
        _file: file,
    }))
}

/// 映射成功时返回可交给 `FontData::from_blob` 的共享字节。
pub fn font_blob(path: &Path) -> Option<Blob> {
    let mapped = map_read_only(path)?;
    let blob: Blob = mapped;
    Some(blob)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn system_font() -> Option<std::path::PathBuf> {
        let windir = std::env::var_os("WINDIR")?;
        let path = std::path::PathBuf::from(windir)
            .join("Fonts")
            .join("msyh.ttc");
        path.is_file().then_some(path)
    }

    #[test]
    fn a_mapped_font_is_byte_identical_to_the_file() {
        let Some(path) = system_font() else {
            eprintln!("跳过：本机没有 msyh.ttc");
            return;
        };
        let mapped = map_read_only(&path).expect("映射系统字体");
        let read = std::fs::read(&path).expect("整读系统字体");
        assert_eq!(mapped.len(), read.len(), "映射长度必须等于文件长度");
        assert_eq!(mapped.bytes(), read.as_slice(), "映射内容必须逐字节一致");
    }

    #[test]
    fn the_blob_keeps_the_mapping_alive() {
        let Some(path) = system_font() else {
            eprintln!("跳过：本机没有 msyh.ttc");
            return;
        };
        let blob = font_blob(&path).expect("字体映射");
        let length = std::fs::metadata(&path).expect("字体元数据").len() as usize;
        let bytes: &[u8] = (*blob).as_ref();
        assert_eq!(bytes.len(), length, "Blob 必须是整份字体");
        assert_eq!(&bytes[..4], b"ttcf", "ttc 集合头");
    }

    #[test]
    fn a_missing_file_is_not_mappable() {
        let missing = std::env::temp_dir().join("dlssg-fontmap-does-not-exist.ttc");
        assert!(map_read_only(&missing).is_none());
        assert!(font_blob(&missing).is_none());
    }
}
