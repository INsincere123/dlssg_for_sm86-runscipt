//! 黄金样本读取器（M0 产出，见 `rust/fixtures/README.md`）。
//!
//! 只给测试与差分工具用。样本目录由 `CARGO_MANIFEST_DIR` 推导：
//! `rust/crates/launcher-core` → `rust/` → `fixtures/`。
//!
//! 读取时**不做任何换行或编码归一化**：样本的全部意义就是逐字节比对。

use std::path::PathBuf;

/// 样本根目录。不做存在性检查，供需要自己判断的调用方使用。
pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("launcher-core 必须位于 rust/crates/ 下")
        .join("fixtures")
}

/// 绝对路径，缺失时直接失败并指出补救方式。
pub fn path(relative: &str) -> PathBuf {
    let base = root();
    let candidate = base.join(relative);
    assert!(
        candidate.exists(),
        "缺少黄金样本 {}（期望在 {}）。运行 rust\\tools\\gen-fixtures.ps1 重新生成。",
        relative,
        base.display()
    );
    candidate
}

pub fn exists(relative: &str) -> bool {
    root().join(relative).exists()
}

/// 按原始字节读取，不修改换行。
pub fn read_bytes(relative: &str) -> Vec<u8> {
    let target = path(relative);
    std::fs::read(&target).unwrap_or_else(|error| panic!("读取 {} 失败：{error}", target.display()))
}

/// 按 UTF-8 读取。样本都是 UTF-8；无 BOM 的普通文本。
pub fn read_text(relative: &str) -> String {
    let bytes = read_bytes(relative);
    String::from_utf8(bytes)
        .unwrap_or_else(|error| panic!("样本 {relative} 不是合法 UTF-8：{error}"))
}

/// 列出某个子目录下的样本相对路径（`/` 分隔，已排序）。
pub fn list(relative: &str) -> Vec<String> {
    let base = root();
    let directory = base.join(relative);
    let mut found = Vec::new();
    collect(&base, &directory, &mut found);
    found.sort();
    found
}

fn collect(base: &std::path::Path, directory: &std::path::Path, out: &mut Vec<String>) {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => panic!("读取样本目录 {} 失败：{error}", directory.display()),
    };
    for entry in entries {
        let entry = entry.expect("读取样本目录条目失败");
        let path = entry.path();
        if path.is_dir() {
            collect(base, &path, out);
        } else if let Ok(relative) = path.strip_prefix(base) {
            out.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_points_at_rust_fixtures() {
        let root = root();
        assert!(root.ends_with("fixtures"), "实际：{}", root.display());
        assert!(root.join("manifest.json").is_file(), "缺少 manifest.json");
    }

    #[test]
    fn text_is_read_without_line_ending_normalisation() {
        let text = read_text("diagnostic/wuthering/default.ini");
        let lone_lf = text.matches('\n').count() - text.matches("\r\n").count();
        assert_eq!(lone_lf, 0, "样本应保持纯 CRLF，实际有 {lone_lf} 个裸 LF");
        assert!(text.contains("\r\n"), "样本应有 CRLF");
    }

    #[test]
    fn list_returns_relative_slash_paths() {
        let files = list("templates/wuthering");
        assert_eq!(
            files,
            vec![
                "templates/wuthering/0.3.2.ini",
                "templates/wuthering/0.4.0-mode-off.ini",
                "templates/wuthering/0.4.0.ini",
                "templates/wuthering/0.4.2.ini",
            ]
        );
    }
}
