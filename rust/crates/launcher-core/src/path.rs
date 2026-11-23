//! 路径归一化与文本哈希（Phase 1 步 2）。
//!
//! 对应现有实现：
//! - `Get-NormalizedPath`（三家游戏脚本各一份，正文逐字相同）：
//!   `[IO.Path]::GetFullPath($Path).TrimEnd('\').ToLowerInvariant()`
//! - `Get-HashOrNull` / `Get-FileHash -Algorithm SHA256`：大写十六进制
//! - `Get-JournalTextHash`（`DeploymentJournal.ps1:228-233`）：UTF-8 文本的 SHA256
//!
//! **归一化只做词法处理**：不解析链接、不访问磁盘、不展开 `%VAR%`。
//! 这一点由样本 `paths/normalized-path.json` 锁定 —— `%TEMP%` 会当作普通目录名
//! 拼到当前工作目录下，而不是展开成环境变量。
//!
//! 唯一需要"环境"的是相对路径的基准：由 [`PathContext`] 显式提供，
//! 因此单元测试可以用样本里记录的工作目录复现 PS 的结果。

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// 相对路径解析所需的基准。
#[derive(Debug, Clone)]
pub struct PathContext {
    cwd: PathBuf,
}

impl PathContext {
    /// 用进程当前目录作为基准。
    pub fn current() -> Result<Self> {
        let cwd = std::env::current_dir().map_err(Error::io_plain)?;
        Ok(Self { cwd })
    }

    pub fn with_working_directory(cwd: impl Into<PathBuf>) -> Self {
        Self { cwd: cwd.into() }
    }

    pub fn working_directory(&self) -> &Path {
        &self.cwd
    }

    /// 等价 `[IO.Path]::GetFullPath($Path)`：保留大小写与结尾分隔符。
    pub fn full_path(&self, input: &str) -> Result<String> {
        assemble(input, &self.cwd)
    }

    /// 等价 `[IO.Path]::GetFullPath($Path).TrimEnd('\').ToLowerInvariant()`。
    pub fn normalize(&self, input: &str) -> Result<String> {
        Ok(trim_trailing_separators(&self.full_path(input)?).to_lowercase())
    }

    /// 归一化一个路径对象；失败时保留原始输入便于日志定位。
    pub fn normalize_path(&self, input: &Path) -> Result<String> {
        let text = input
            .to_str()
            .ok_or_else(|| Error::invalid(format!("路径不是合法 UTF-8：{}", input.display())))?;
        self.normalize(text)
    }
}

/// 用进程当前目录归一化。等价 PS 的 `Get-NormalizedPath`。
pub fn normalize(input: &str) -> Result<String> {
    PathContext::current()?.normalize(input)
}

/// 归一化实现本体。`cwd` 只用于相对路径、驱动器相对路径与无盘符的根路径。
fn assemble(input: &str, cwd: &Path) -> Result<String> {
    if input.is_empty() {
        return Err(Error::invalid("路径为空"));
    }
    let unified = input.replace('/', "\\");

    // 1) 扩展长度前缀：保留前缀，不折叠 `..`（Win32 语义）。
    if let Some(rest) = unified.strip_prefix("\\\\?\\") {
        if rest.is_empty() {
            return Err(Error::invalid(format!("扩展路径缺少主体：{input}")));
        }
        let body = match rest.strip_prefix("UNC\\") {
            Some(unc) => format!("\\\\{unc}"),
            None => rest.to_owned(),
        };
        return Ok(format!("\\\\?\\{}", body));
    }

    // 2) 拆分：根 + 其余段，再折叠 `.` / `..`。
    let (root, rest) = split_root(&unified, cwd)?;
    let mut segments: Vec<String> = Vec::new();
    for segment in rest.split('\\') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other.to_owned()),
        }
    }

    let mut assembled = root;
    for segment in &segments {
        if !assembled.ends_with('\\') {
            assembled.push('\\');
        }
        assembled.push_str(segment);
    }
    Ok(assembled)
}

/// 去掉根之后的部分，返回（根前缀，其余段）。
fn split_root(unified: &str, cwd: &Path) -> Result<(String, String)> {
    // UNC：\\server\share
    if let Some(rest) = unified.strip_prefix("\\\\") {
        let mut parts = rest.splitn(3, '\\');
        let server = parts.next().unwrap_or_default();
        let share = parts.next().unwrap_or_default();
        if server.is_empty() || share.is_empty() {
            return Err(Error::invalid(format!(
                "UNC 路径缺少服务器或共享名：{unified}"
            )));
        }
        let rest = parts.next().unwrap_or_default().to_owned();
        return Ok((format!("\\\\{server}\\{share}\\"), rest));
    }

    // 盘符：`X:`、`X:\...`、`X:relative`
    let bytes = unified.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        let drive = bytes[0] as char;
        let after = &unified[2..];
        if after.is_empty() {
            // `C:` 是驱动器相对路径。同一驱动器用工作目录；其他驱动器用该盘根目录
            // （Windows 里从未作为工作目录用过的驱动器，其 per-drive CWD 就是根）。
            return Ok((drive_base(cwd, drive), String::new()));
        }
        if let Some(rest) = after.strip_prefix('\\') {
            return Ok((format!("{drive}:\\"), rest.to_owned()));
        }
        return Ok((drive_base(cwd, drive), after.to_owned()));
    }

    // 无盘符的根：`\foo` → 当前驱动器根。
    if let Some(rest) = unified.strip_prefix('\\') {
        let drive = cwd
            .to_string_lossy()
            .chars()
            .next()
            .ok_or_else(|| Error::invalid("工作目录为空，无法解析根路径"))?;
        return Ok((format!("{drive}:\\"), rest.to_owned()));
    }

    // 纯相对路径。
    Ok((ensure_trailing_separator(cwd), unified.to_owned()))
}

/// 驱动器相对路径的基准，总是以 `\` 结尾。
fn drive_base(cwd: &Path, drive: char) -> String {
    match same_drive(cwd, drive) {
        true => ensure_trailing_separator(cwd),
        false => format!("{drive}:\\"),
    }
}

fn same_drive(cwd: &Path, drive: char) -> bool {
    cwd.to_string_lossy()
        .chars()
        .next()
        .is_some_and(|first| first.eq_ignore_ascii_case(&drive))
}

fn ensure_trailing_separator(path: &Path) -> String {
    let text = path.to_string_lossy().replace('/', "\\");
    if text.ends_with('\\') {
        text
    } else {
        format!("{text}\\")
    }
}

fn trim_trailing_separators(path: &str) -> String {
    let trimmed = path.trim_end_matches('\\');
    if trimmed.is_empty() {
        path.to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// 等价 `Get-FileHash -Algorithm SHA256`：大写十六进制。
pub fn sha256_upper(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        text.push_str(&format!("{byte:02X}"));
    }
    text
}

/// 等价 `Get-JournalTextHash`：UTF-8 编码后取 SHA256。
pub fn text_hash_upper(text: &str) -> String {
    sha256_upper(text.as_bytes())
}

/// 等价 `Get-HashOrNull`：文件不存在时返回 `None`，而不是错误。
pub fn file_hash_or_none(path: &Path) -> Result<Option<String>> {
    match std::fs::metadata(path) {
        Ok(metadata) => {
            if metadata.is_dir() {
                return Err(Error::invalid(format!(
                    "目标是目录，不能取哈希：{}",
                    path.display()
                )));
            }
            let bytes = std::fs::read(path).map_err(|error| Error::io(path, error))?;
            Ok(Some(sha256_upper(&bytes)))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Error::io(path, error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_context() -> PathContext {
        let raw = crate::fixtures::read_text("paths/normalized-path.json");
        let parsed: serde_json::Value = serde_json::from_str(&raw).expect("样本 JSON 应可解析");
        let cwd = parsed["WorkingDirectory"]
            .as_str()
            .expect("样本缺少 WorkingDirectory");
        PathContext::with_working_directory(cwd)
    }

    #[test]
    fn matches_every_recorded_case() {
        let raw = crate::fixtures::read_text("paths/normalized-path.json");
        let parsed: serde_json::Value = serde_json::from_str(&raw).expect("样本 JSON 应可解析");
        let context = sample_context();
        let cases = parsed["Cases"].as_array().expect("样本缺少 Cases");
        assert!(cases.len() >= 13, "样本用例数异常：{}", cases.len());
        for case in cases {
            let input = case["Input"].as_str().expect("缺少 Input");
            let expected = case["Expected"].as_str().expect("缺少 Expected");
            let actual = context.normalize(input).expect("归一化不应失败");
            assert_eq!(actual, expected, "输入：{input}");
        }
    }

    #[test]
    fn empty_input_is_rejected() {
        let context = sample_context();
        assert!(context.normalize("").is_err());
    }

    #[test]
    fn trailing_separators_are_trimmed_once() {
        let context = sample_context();
        assert_eq!(context.normalize("D:\\").expect("归一化"), "d:");
        assert_eq!(context.normalize("D:\\a\\").expect("归一化"), "d:\\a");
        assert_eq!(context.normalize("D:/a/b/").expect("归一化"), "d:\\a\\b");
    }

    #[test]
    fn dot_segments_stop_at_the_root() {
        let context = sample_context();
        assert_eq!(
            context.normalize("D:\\a\\..\\..\\..\\b").expect("归一化"),
            "d:\\b"
        );
    }

    #[test]
    fn environment_variables_are_not_expanded() {
        let context = sample_context();
        assert_eq!(
            context.normalize("%TEMP%\\x.ini").expect("归一化"),
            "d:\\games\\dlssg_for_sm86\\%temp%\\x.ini"
        );
    }

    #[test]
    fn samba_and_extended_prefixes_keep_their_shape() {
        let context = sample_context();
        assert_eq!(
            context
                .normalize("\\\\server\\share\\Game")
                .expect("归一化"),
            "\\\\server\\share\\game"
        );
        assert_eq!(
            context.normalize("\\\\?\\D:\\Games\\Foo").expect("归一化"),
            "\\\\?\\d:\\games\\foo"
        );
    }

    #[test]
    fn sha256_is_upper_hex_and_matches_known_vectors() {
        assert_eq!(
            sha256_upper(b""),
            "E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855"
        );
        assert_eq!(
            sha256_upper(b"abc"),
            "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"
        );
    }

    #[test]
    fn text_hash_hashes_utf8_bytes() {
        assert_eq!(text_hash_upper("abc"), sha256_upper(b"abc"));
        assert_eq!(
            text_hash_upper("黑神话：悟空"),
            sha256_upper("黑神话：悟空".as_bytes())
        );
    }

    #[test]
    fn file_hash_returns_none_for_missing_file() {
        let missing = std::env::temp_dir().join("dlssg-core-missing-file.ini");
        let _ = std::fs::remove_file(&missing);
        assert_eq!(file_hash_or_none(&missing).expect("不应报错"), None);
    }

    #[test]
    fn file_hash_matches_fixture_bytes() {
        let relative = "templates/wuthering/0.4.2.ini";
        let bytes = crate::fixtures::read_bytes(relative);
        let path = crate::fixtures::path(relative);
        assert_eq!(
            file_hash_or_none(&path).expect("哈希"),
            Some(sha256_upper(&bytes))
        );
    }
}
