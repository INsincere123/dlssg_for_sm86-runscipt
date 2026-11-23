//! 便携包清单（M5-02）。
//!
//! 对齐 `启动脚本/scripts/tools/Export-PortableLauncher.ps1` 的 `package-manifest.json`：
//!
//! ```json
//! { "FormatVersion": 1, "PackageType": 1,
//!   "Files": [ { "Path": "dlssg_sm86/runtime/x.dll", "Length": 123, "SHA256": "ABCD…" } ] }
//! ```
//!
//! 相对路径统一用 `/`，`SHA256` 大写十六进制（与 `Get-FileHash` 一致）。
//!
//! 新程序把包拆成两份（PARITY 12.4）：`dlssg_sm86/sr/**` 属于 **sr 配件包**，其余属于**基础包**。
//! 因此每个条目多一个 `Package` 字段（`base` / `sr`），旧工具读到的其余字段不变（记为 D-10）。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// 清单格式版本。
pub const FORMAT_VERSION: u32 = 1;

/// 包类型：1 = 完整（含 Steam 入口），与 PS 的 `PackageType` 取值一致。
pub const PACKAGE_TYPE_FULL: u8 = 1;

/// 清单文件名。
pub const MANIFEST_FILE: &str = "package-manifest.json";

/// 条目属于哪个包。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Package {
    Base,
    Sr,
}

impl Package {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Sr => "sr",
        }
    }
}

/// 清单里的一个文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// 相对路径，`/` 分隔。
    #[serde(rename = "Path")]
    pub path: String,
    #[serde(rename = "Length")]
    pub length: u64,
    /// 大写十六进制。
    #[serde(rename = "SHA256")]
    pub sha256: String,
    #[serde(rename = "Package", default = "default_package")]
    pub package: Package,
}

fn default_package() -> Package {
    Package::Base
}

/// `package-manifest.json` 的内容。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageManifest {
    #[serde(rename = "FormatVersion")]
    pub format_version: u32,
    #[serde(rename = "PackageType")]
    pub package_type: u8,
    #[serde(rename = "Files")]
    pub files: Vec<ManifestEntry>,
}

/// 不在便携包里的目录/文件（本机状态、日志、导出产物、Git 元数据）。
pub const EXCLUDED: [&str; 7] = [
    ".state",
    "logs",
    "dist",
    "runs",
    ".git",
    "preferences.json",
    ".gitmodules",
];

/// 判断相对路径是否属于排除项（任一层级命中即排除）。
pub fn is_excluded(relative: &str) -> bool {
    let normalized = relative.replace('\\', "/");
    normalized
        .split('/')
        .any(|segment| EXCLUDED.contains(&segment))
        || normalized.ends_with(".tmp")
        || normalized.ends_with(".tmp.previous")
}

/// `dlssg_sm86/sr/**` 归配件包，其余归基础包。
pub fn classify(relative: &str) -> Package {
    let normalized = relative.replace('\\', "/").to_ascii_lowercase();
    if normalized.starts_with("dlssg_sm86/sr/") {
        Package::Sr
    } else {
        Package::Base
    }
}

impl PackageManifest {
    /// 由根目录收集清单：跳过排除项与清单自身，按路径排序。
    pub fn collect(root: &Path) -> Result<Self> {
        let mut files = Vec::new();
        walk(root, root, &mut files)?;
        files.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(Self {
            format_version: FORMAT_VERSION,
            package_type: PACKAGE_TYPE_FULL,
            files,
        })
    }

    /// 某个包的条目。
    pub fn package(&self, package: Package) -> Vec<&ManifestEntry> {
        self.files
            .iter()
            .filter(|entry| entry.package == package)
            .collect()
    }

    /// 某个包的字节数。
    pub fn package_bytes(&self, package: Package) -> u64 {
        self.package(package).iter().map(|entry| entry.length).sum()
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|error| Error::invalid(format!("清单无法序列化：{error}")))
    }

    pub fn from_json(text: &str) -> Result<Self> {
        let manifest: Self = serde_json::from_str(text.trim())
            .map_err(|error| Error::invalid(format!("清单无法解析：{error}")))?;
        if manifest.format_version != FORMAT_VERSION {
            return Err(Error::invalid(format!(
                "清单格式版本不受支持：{}（期望 {FORMAT_VERSION}）",
                manifest.format_version
            )));
        }
        Ok(manifest)
    }

    /// 写清单文件（UTF-8 无 BOM，末尾换行）。
    pub fn write_to(&self, directory: &Path) -> Result<PathBuf> {
        let path = directory.join(MANIFEST_FILE);
        let mut text = self.to_json()?;
        text.push('\n');
        std::fs::write(&path, text.as_bytes()).map_err(|error| Error::io(&path, error))?;
        Ok(path)
    }

    /// 读清单文件。
    pub fn read_from(directory: &Path) -> Result<Self> {
        let path = directory.join(MANIFEST_FILE);
        let text = std::fs::read_to_string(&path).map_err(|error| Error::io(&path, error))?;
        Self::from_json(&text)
    }
}

fn walk(root: &Path, directory: &Path, files: &mut Vec<ManifestEntry>) -> Result<()> {
    let entries = std::fs::read_dir(directory).map_err(|error| Error::io(directory, error))?;
    for entry in entries {
        let entry = entry.map_err(|error| Error::io(directory, error))?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map(|value| value.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| path.to_string_lossy().into_owned());
        if relative == MANIFEST_FILE || is_excluded(&relative) {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| Error::io(&path, error))?;
        if metadata.file_type().is_symlink() {
            // 链接不进包：便携包必须自包含，链接指向本机路径没有意义。
            continue;
        }
        if metadata.is_dir() {
            walk(root, &path, files)?;
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|error| Error::io(&path, error))?;
        files.push(ManifestEntry {
            path: relative.clone(),
            length: bytes.len() as u64,
            sha256: crate::path::sha256_upper(&bytes),
            package: classify(&relative),
        });
    }
    Ok(())
}

/// 自检发现的一个问题。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// 清单里有、磁盘上没有。
    Missing { path: String },
    /// 大小不符。
    SizeMismatch {
        path: String,
        expected: u64,
        actual: u64,
    },
    /// 大小相符但内容变了。
    HashMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    /// 磁盘上有、清单里没有。
    Extra { path: String },
}

impl Problem {
    pub fn path(&self) -> &str {
        match self {
            Self::Missing { path }
            | Self::SizeMismatch { path, .. }
            | Self::HashMismatch { path, .. }
            | Self::Extra { path } => path,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Missing { path } => format!("缺失：{path}"),
            Self::SizeMismatch {
                path,
                expected,
                actual,
            } => format!("大小不符：{path}（清单 {expected} B，实际 {actual} B）"),
            Self::HashMismatch { path, .. } => format!("内容被改动：{path}"),
            Self::Extra { path } => format!("多出文件：{path}"),
        }
    }
}

/// 一次自检的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerifyReport {
    /// 实际核对了多少个清单条目。
    pub checked: usize,
    pub problems: Vec<Problem>,
}

impl VerifyReport {
    pub fn is_clean(&self) -> bool {
        self.problems.is_empty()
    }

    pub fn summary(&self) -> String {
        if self.is_clean() {
            return format!("校验通过：{} 个文件与清单一致。", self.checked);
        }
        let mut text = format!(
            "校验失败：核对 {} 个文件，发现 {} 个问题。",
            self.checked,
            self.problems.len()
        );
        for problem in &self.problems {
            text.push_str("\r\n- ");
            text.push_str(&problem.describe());
        }
        text
    }
}

/// 按清单校验目录：大小、SHA256、缺失，以及清单里没有的多余文件。
///
/// 只比对清单声明的文件；排除项（`.state`、`logs` 等）不参与"多余"判定。
pub fn verify(root: &Path, manifest: &PackageManifest) -> Result<VerifyReport> {
    verify_package(root, manifest, None)
}

/// 只校验某一个包（`Some(Package::Sr)` 时不检查基础包条目）。
pub fn verify_package(
    root: &Path,
    manifest: &PackageManifest,
    only: Option<Package>,
) -> Result<VerifyReport> {
    let mut report = VerifyReport::default();
    let mut declared: Vec<&str> = Vec::new();
    for entry in &manifest.files {
        if let Some(package) = only {
            if entry.package != package {
                continue;
            }
        }
        declared.push(&entry.path);
        let path = root.join(entry.path.replace('/', "\\"));
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                report.problems.push(Problem::Missing {
                    path: entry.path.clone(),
                });
                continue;
            }
            Err(error) => return Err(Error::io(&path, error)),
        };
        report.checked += 1;
        if bytes.len() as u64 != entry.length {
            report.problems.push(Problem::SizeMismatch {
                path: entry.path.clone(),
                expected: entry.length,
                actual: bytes.len() as u64,
            });
            continue;
        }
        let actual = crate::path::sha256_upper(&bytes);
        if !actual.eq_ignore_ascii_case(&entry.sha256) {
            report.problems.push(Problem::HashMismatch {
                path: entry.path.clone(),
                expected: entry.sha256.clone(),
                actual,
            });
        }
    }

    // 多余文件：磁盘上有、清单里没有（清单自身与排除项不算）。
    if only.is_none() {
        let current = PackageManifest::collect(root)?;
        for entry in current.files {
            if !declared.contains(&entry.path.as_str()) {
                report.problems.push(Problem::Extra { path: entry.path });
            }
        }
    }
    Ok(report)
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
        let root = std::env::temp_dir().join(format!("dlssg-export-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("根目录");
        Tree(root)
    }

    fn write(root: &Path, relative: &str, content: &[u8]) {
        let path = root.join(relative.replace('/', "\\"));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("父目录");
        }
        std::fs::write(&path, content).expect("写文件");
    }

    #[test]
    fn classification_splits_the_sr_accessory() {
        assert_eq!(classify("dlssg_sm86/sr/sr_kernels.dll"), Package::Sr);
        assert_eq!(classify("dlssg_sm86\\SR\\nvngx_dlss.dll"), Package::Sr);
        assert_eq!(
            classify("dlssg_sm86/runtime/nvngx_dlssg.dll"),
            Package::Base
        );
        assert_eq!(classify("启动脚本/启动鸣潮.bat"), Package::Base);
        assert_eq!(classify("dlssg_sm86/srx/other.dll"), Package::Base);
    }

    #[test]
    fn exclusions_cover_state_logs_and_export_artifacts() {
        for relative in [
            "启动脚本/.state/steam-1/active.json",
            "启动脚本/logs/2026.log",
            "dist/package.zip",
            "启动脚本/runs/abc/request.ini",
            ".git/config",
            "启动脚本/preferences.json",
            "dlssg_sm86/a.tmp",
        ] {
            assert!(is_excluded(relative), "应排除：{relative}");
        }
        for relative in ["dlssg_sm86/runtime/x.dll", "启动脚本/启动异环.bat"] {
            assert!(!is_excluded(relative), "不应排除：{relative}");
        }
    }

    #[test]
    fn collect_records_size_hash_package_and_skips_excluded() {
        let tree = tree("collect");
        write(&tree.0, "dlssg_sm86/runtime/a.dll", b"abc");
        write(&tree.0, "dlssg_sm86/sr/sr_kernels.dll", b"12345");
        write(&tree.0, "启动脚本/.state/x/active.json", b"{}");
        write(&tree.0, "启动脚本/logs/run.log", b"log");

        let manifest = PackageManifest::collect(&tree.0).expect("收集");
        assert_eq!(manifest.format_version, FORMAT_VERSION);
        assert_eq!(manifest.package_type, PACKAGE_TYPE_FULL);
        assert_eq!(manifest.files.len(), 2, "{:?}", manifest.files);
        assert_eq!(manifest.files[0].path, "dlssg_sm86/runtime/a.dll");
        assert_eq!(manifest.files[0].length, 3);
        assert_eq!(manifest.files[0].sha256, crate::path::sha256_upper(b"abc"));
        assert_eq!(manifest.files[1].package, Package::Sr);
        assert_eq!(manifest.package_bytes(Package::Sr), 5);
        assert_eq!(manifest.package_bytes(Package::Base), 3);
    }

    #[test]
    fn json_uses_pascal_case_keys_and_round_trips() {
        let tree = tree("json");
        write(&tree.0, "dlssg_sm86/runtime/a.dll", b"abc");
        let manifest = PackageManifest::collect(&tree.0).expect("收集");
        let json = manifest.to_json().expect("序列化");
        assert!(json.contains("\"FormatVersion\": 1"), "{json}");
        assert!(json.contains("\"PackageType\": 1"), "{json}");
        assert!(json.contains("\"SHA256\""), "{json}");
        assert!(
            json.contains("\"Path\": \"dlssg_sm86/runtime/a.dll\""),
            "{json}"
        );
        assert_eq!(PackageManifest::from_json(&json).expect("解析"), manifest);
    }

    #[test]
    fn writing_and_reading_the_manifest_file() {
        let tree = tree("file");
        write(&tree.0, "dlssg_sm86/runtime/a.dll", b"abc");
        let manifest = PackageManifest::collect(&tree.0).expect("收集");
        let path = manifest.write_to(&tree.0).expect("写清单");
        assert!(path.ends_with(MANIFEST_FILE));
        let text = std::fs::read_to_string(&path).expect("读");
        assert!(text.ends_with('\n'), "清单末尾应有换行");
        assert_eq!(
            PackageManifest::read_from(&tree.0).expect("读清单"),
            manifest
        );
        // 清单自身不进清单
        let again = PackageManifest::collect(&tree.0).expect("再收集");
        assert!(!again.files.iter().any(|entry| entry.path == MANIFEST_FILE));
    }

    #[test]
    fn unknown_format_version_is_rejected() {
        let text = r#"{"FormatVersion":2,"PackageType":1,"Files":[]}"#;
        let error = PackageManifest::from_json(text).expect_err("应拒绝");
        assert!(error.to_string().contains("格式版本"), "{error}");
    }

    #[test]
    fn entries_without_package_default_to_base() {
        let text = r#"{"FormatVersion":1,"PackageType":1,"Files":[{"Path":"a.dll","Length":1,"SHA256":"AB"}]}"#;
        let manifest = PackageManifest::from_json(text).expect("解析");
        assert_eq!(manifest.files[0].package, Package::Base);
    }
}

#[cfg(test)]
mod verify_tests {
    use super::*;

    struct Tree(std::path::PathBuf);

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tree(name: &str) -> Tree {
        let root = std::env::temp_dir().join(format!("dlssg-verify-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("根目录");
        Tree(root)
    }

    fn write(root: &Path, relative: &str, content: &[u8]) {
        let path = root.join(relative.replace('/', "\\"));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("父目录");
        }
        std::fs::write(&path, content).expect("写文件");
    }

    fn sample(name: &str) -> (Tree, PackageManifest) {
        let tree = tree(name);
        write(&tree.0, "dlssg_sm86/runtime/a.dll", b"abc");
        write(&tree.0, "dlssg_sm86/sr/sr_kernels.dll", b"12345");
        let manifest = PackageManifest::collect(&tree.0).expect("收集");
        (tree, manifest)
    }

    #[test]
    fn clean_tree_passes() {
        let (tree, manifest) = sample("clean");
        let report = verify(&tree.0, &manifest).expect("校验");
        assert!(report.is_clean(), "{}", report.summary());
        assert_eq!(report.checked, 2);
        assert!(report.summary().contains("校验通过"));
    }

    #[test]
    fn tampered_content_is_detected() {
        let (tree, manifest) = sample("tamper");
        write(&tree.0, "dlssg_sm86/runtime/a.dll", b"xyz");
        let report = verify(&tree.0, &manifest).expect("校验");
        assert!(!report.is_clean());
        assert!(
            matches!(report.problems.as_slice(), [Problem::HashMismatch { path, .. }] if path == "dlssg_sm86/runtime/a.dll"),
            "{:?}",
            report.problems
        );
        assert!(report.summary().contains("内容被改动"));
    }

    #[test]
    fn size_change_is_reported_before_hash() {
        let (tree, manifest) = sample("size");
        write(&tree.0, "dlssg_sm86/sr/sr_kernels.dll", b"1234567890");
        let report = verify(&tree.0, &manifest).expect("校验");
        assert!(
            matches!(
                report.problems.as_slice(),
                [Problem::SizeMismatch {
                    expected: 5,
                    actual: 10,
                    ..
                }]
            ),
            "{:?}",
            report.problems
        );
    }

    #[test]
    fn missing_file_is_detected() {
        let (tree, manifest) = sample("missing");
        std::fs::remove_file(tree.0.join("dlssg_sm86/runtime/a.dll")).expect("删");
        let report = verify(&tree.0, &manifest).expect("校验");
        assert!(
            matches!(report.problems.as_slice(), [Problem::Missing { path }] if path == "dlssg_sm86/runtime/a.dll"),
            "{:?}",
            report.problems
        );
    }

    #[test]
    fn extra_file_is_detected_but_excluded_paths_are_not() {
        let (tree, manifest) = sample("extra");
        write(&tree.0, "dlssg_sm86/runtime/injected.dll", b"new");
        write(&tree.0, "dlssg_sm86/logs/run.log", b"log");
        write(&tree.0, "启动脚本/.state/x/active.json", b"{}");
        let report = verify(&tree.0, &manifest).expect("校验");
        assert!(
            matches!(report.problems.as_slice(), [Problem::Extra { path }] if path == "dlssg_sm86/runtime/injected.dll"),
            "{:?}",
            report.problems
        );
    }

    #[test]
    fn package_scoped_verification_ignores_the_other_package() {
        let (tree, manifest) = sample("package");
        write(&tree.0, "dlssg_sm86/runtime/a.dll", b"changed");
        let base = verify_package(&tree.0, &manifest, Some(Package::Base)).expect("基础包");
        assert_eq!(base.checked, 1);
        assert!(!base.is_clean(), "基础包应发现问题");
        let sr = verify_package(&tree.0, &manifest, Some(Package::Sr)).expect("sr 包");
        assert_eq!(sr.checked, 1);
        assert!(sr.is_clean(), "sr 包本身没问题：{}", sr.summary());
    }
}
