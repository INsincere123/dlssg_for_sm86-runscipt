//! 统一错误类型。
//!
//! 现有 PowerShell 实现用 `throw "字符串"` 传播失败；Rust 侧需要能区分
//! "IO 失败 / 配置文本错误 / 前置条件不满足 / 互斥超时 / 记录不兼容"，
//! 因为恢复流程对不同类别的处理不同（冲突要保留现场，配置错误要停止部署）。
//!
//! 错误文案不在 PARITY 的等价范围内（只有文件内容与命名需要逐字节一致），
//! 因此这里用中文写清原因，便于直接进日志。

use std::fmt::{self, Display};
use std::io;
use std::path::{Path, PathBuf};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    /// 文件系统操作失败。`path` 保留失败当时的路径，便于日志定位。
    Io {
        path: Option<PathBuf>,
        source: io::Error,
    },
    /// 配置文本错误。`line` 为 1 起始行号；整体性错误时为 `None`。
    Config {
        line: Option<usize>,
        message: String,
    },
    /// 前置条件不满足（等价于 PS 里的 `throw`）：记录已存在、路径被替换、链接类型不符等。
    Invalid { message: String },
    /// 在等待窗口内没有拿到互斥。
    MutexTimeout { name: String, wait_ms: u64 },
    /// 记录或格式不受支持，继续处理会破坏现场。
    Unsupported { what: String, value: String },
    /// 当前环境做不到（例如没有创建符号链接的特权）。
    Unavailable { what: String },
}

impl Error {
    pub fn io(path: impl AsRef<Path>, source: io::Error) -> Self {
        Error::Io {
            path: Some(path.as_ref().to_path_buf()),
            source,
        }
    }

    pub fn io_plain(source: io::Error) -> Self {
        Error::Io { path: None, source }
    }

    pub fn config(line: usize, message: impl Into<String>) -> Self {
        Error::Config {
            line: Some(line),
            message: message.into(),
        }
    }

    pub fn config_plain(message: impl Into<String>) -> Self {
        Error::Config {
            line: None,
            message: message.into(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Error::Invalid {
            message: message.into(),
        }
    }

    pub fn unsupported(what: impl Into<String>, value: impl Into<String>) -> Self {
        Error::Unsupported {
            what: what.into(),
            value: value.into(),
        }
    }

    pub fn unavailable(what: impl Into<String>) -> Self {
        Error::Unavailable { what: what.into() }
    }

    /// 是否属于"现场必须保留、不能继续清理"的错误类别。
    ///
    /// 恢复流程用这个判断决定是否保留记录：IO 与前置条件失败都要保留，
    /// 互斥超时说明另一进程正在操作，同样保留。
    pub fn must_preserve_site(&self) -> bool {
        !matches!(self, Error::Config { .. })
    }
}

impl Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, source } => match path {
                Some(path) => write!(formatter, "文件操作失败 {}：{}", path.display(), source),
                None => write!(formatter, "文件操作失败：{source}"),
            },
            Error::Config { line, message } => match line {
                Some(line) => write!(formatter, "配置第 {line} 行：{message}"),
                None => write!(formatter, "配置错误：{message}"),
            },
            Error::Invalid { message } => write!(formatter, "{message}"),
            Error::MutexTimeout { name, wait_ms } => {
                write!(formatter, "等待互斥超时（{wait_ms} ms）：{name}")
            }
            Error::Unsupported { what, value } => {
                write!(formatter, "不支持的{what}：{value}")
            }
            Error::Unavailable { what } => write!(formatter, "当前环境无法完成：{what}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(source: io::Error) -> Self {
        Error::io_plain(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_error_carries_line_number() {
        let text = Error::config(7, "缺少等号").to_string();
        assert!(text.contains("第 7 行"), "实际：{text}");
        assert!(text.contains("缺少等号"), "实际：{text}");
    }

    #[test]
    fn io_error_keeps_path_and_source() {
        let error = Error::io(
            "C:\\games\\Engine.ini",
            io::Error::new(io::ErrorKind::PermissionDenied, "denied"),
        );
        let text = error.to_string();
        assert!(text.contains("Engine.ini"), "实际：{text}");
        assert!(text.contains("denied"), "实际：{text}");
        assert!(std::error::Error::source(&error).is_some());
    }

    #[test]
    fn only_config_errors_may_drop_the_site() {
        assert!(!Error::config_plain("坏的键").must_preserve_site());
        assert!(Error::invalid("链接已改变").must_preserve_site());
        assert!(Error::unavailable("缺少特权").must_preserve_site());
        assert!(
            Error::MutexTimeout {
                name: "Global\\x".into(),
                wait_ms: 0
            }
            .must_preserve_site()
        );
    }

    #[test]
    fn io_errors_convert_via_question_mark() {
        fn fail() -> Result<()> {
            Err(io::Error::new(io::ErrorKind::NotFound, "missing"))?;
            Ok(())
        }
        assert!(matches!(fail(), Err(Error::Io { .. })));
    }
}
