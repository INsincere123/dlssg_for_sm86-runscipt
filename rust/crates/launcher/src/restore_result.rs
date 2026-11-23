//! 「恢复原状」结果解析（界面弹窗用）。
//!
//! 恢复真正在提权后的引擎子进程里跑，界面只看它写的报告：
//!
//! - 成功：`{"code": "restored", "game": "...", "message": "..."}`
//! - 失败：`{"error": "..."}`
//!
//! 报告是手写 JSON（不带反斜杠转义以外的复杂结构），这里只认这两个键，
//! 其它内容一律当作"还不能判定"，让界面继续等或按超时处理。
//!
//! 只做字符串定位，不引入 JSON 依赖：报告由本程序自己生成，格式固定。

use std::path::Path;

/// 一次恢复的结论。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreOutcome {
    /// 恢复完成，`message` 是引擎给的说明。
    Restored { message: String },
    /// 恢复失败，`error` 是失败原因。
    Failed { error: String },
}

impl RestoreOutcome {
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Restored { .. })
    }

    /// 弹窗标题。
    pub fn title(&self) -> &'static str {
        if self.is_ok() {
            "恢复原状完成"
        } else {
            "恢复原状失败"
        }
    }

    /// 弹窗正文。
    pub fn detail(&self) -> String {
        match self {
            Self::Restored { message } => {
                if message.trim().is_empty() {
                    "已按记录恢复游戏目录与配置。".to_owned()
                } else {
                    message.clone()
                }
            }
            Self::Failed { error } => {
                format!("{error}\n\n日志里保留了失败步骤；处理后可在该行重试。")
            }
        }
    }
}

/// 解析报告文本；没有结论时返回 `None`。
///
/// 认三种报告：`restore`（`code=restored`）、`stop`（`code=stopped`，带 `restored` 布尔）、
/// 以及失败时的 `{"error": ...}`。
pub fn parse(text: &str) -> Option<RestoreOutcome> {
    if let Some(message) = string_value(text, "error") {
        return Some(RestoreOutcome::Failed { error: message });
    }
    let code = string_value(text, "code")?;
    let message = string_value(text, "message").unwrap_or_default();
    match code.as_str() {
        "restored" => Some(RestoreOutcome::Restored { message }),
        // 「结束并恢复」：杀完进程后可能没有部署记录（无需恢复），那也是一次成功，
        // 具体说明在 message 里，界面照原样显示。
        "stopped" => Some(RestoreOutcome::Restored { message }),
        _ => None,
    }
}

/// 读报告文件；不存在或读不了都返回 `None`。
pub fn read(path: &Path) -> Option<RestoreOutcome> {
    let text = std::fs::read_to_string(path).ok()?;
    parse(&text)
}

/// 取一个顶层字符串值：`"key"` 之后第一个 `:` 再之后的第一个 `"..."`，按需反转义。
fn string_value(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let start = text.find(&needle)? + needle.len();
    let rest = &text[start..];
    let colon = rest.find(':')?;
    let rest = &rest[colon + 1..];
    let quote = rest.find('"')?;
    let rest = &rest[quote + 1..];
    let mut value = String::new();
    let mut escaped = false;
    for character in rest.chars() {
        if escaped {
            value.push(match character {
                'n' => '\n',
                't' => '\t',
                other => other,
            });
            escaped = false;
            continue;
        }
        match character {
            '\\' => escaped = true,
            '"' => return Some(value),
            other => value.push(other),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_report_is_parsed() {
        let text = "{\n  \"code\": \"restored\",\n  \"game\": \"nte\",\n  \"message\": \"已恢复 3 项\"\n}\n";
        let outcome = parse(text).expect("解析");
        assert_eq!(
            outcome,
            RestoreOutcome::Restored {
                message: "已恢复 3 项".to_owned()
            }
        );
        assert_eq!(outcome.title(), "恢复原状完成");
        assert!(outcome.is_ok());
    }

    #[test]
    fn failure_report_is_parsed() {
        let text = "{\n  \"error\": \"恢复失败：备份校验失败\"\n}\n";
        let outcome = parse(text).expect("解析");
        assert_eq!(
            outcome,
            RestoreOutcome::Failed {
                error: "恢复失败：备份校验失败".to_owned()
            }
        );
        assert!(!outcome.is_ok());
        assert_eq!(outcome.title(), "恢复原状失败");
        assert!(outcome.detail().contains("备份校验失败"));
    }

    #[test]
    fn incomplete_reports_are_not_a_conclusion() {
        assert!(parse("").is_none());
        assert!(
            parse("{\n  \"code\": \"executed\"\n}\n").is_none(),
            "跑完启动的报告不算恢复结论"
        );
        assert!(parse("{\"game\":\"nte\"}").is_none());
        assert!(parse("not json").is_none());
    }

    #[test]
    fn stop_report_is_a_success_with_its_own_message() {
        let text = "{\n  \"code\": \"stopped\",\n  \"game\": \"nte\",\n  \"killed\": 2,\n  \"failed\": 0,\n  \"restored\": true,\n  \"message\": \"已结束 2 个进程；引擎已完成恢复\"\n}\n";
        let outcome = parse(text).expect("解析");
        assert!(outcome.is_ok());
        assert!(outcome.detail().contains("已结束 2 个进程"), "{outcome:?}");

        // 没有部署记录时 restored=false 也算"结束成功"，说明来自 message
        let text = "{\"code\": \"stopped\", \"restored\": false, \"message\": \"已结束 0 个进程；没有未完成的部署，无需恢复\"}";
        let outcome = parse(text).expect("解析");
        assert!(outcome.is_ok());
        assert!(outcome.detail().contains("无需恢复"));
    }

    #[test]
    fn missing_message_falls_back_to_a_plain_sentence() {
        let outcome = parse("{\"code\": \"restored\"}").expect("解析");
        assert!(outcome.detail().contains("已按记录恢复"));
    }

    #[test]
    fn escaped_characters_round_trip() {
        let outcome = parse("{\"error\": \"第一行\\n第二行\"}").expect("解析");
        match outcome {
            RestoreOutcome::Failed { error } => assert_eq!(error, "第一行\n第二行"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn missing_file_is_none() {
        let path = std::env::temp_dir().join("dlssg-restore-result-not-here.json");
        let _ = std::fs::remove_file(&path);
        assert!(read(&path).is_none());
    }
}
