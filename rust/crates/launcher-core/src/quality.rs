//! 画质档位与引擎宿主（M3-05）。
//!
//! 对齐 `启动脚本/scripts/common/UnrealQuality.ps1`：
//!
//! - 档位 `Tier1`..`Tier5`，缺省 `Tier2`，非法值报"画质档位无效。"
//! - 分档目录固定：`01-高性能均衡`、`02-主流均衡`、`03-性能优先`、`04-入门流畅`、`05-自定义`
//! - 配置源 `<根>\配置\虚幻通用分类\UE<4|5>\<分档目录>\Engine.ini`
//! - 未确定 UE4/UE5 时报"尚未确定UE4/UE5，请等待识别或在高级设置校准引擎版本。"
//!
//! 与现有实现的差异（PARITY 决策 3.6）：新程序**不做自动识别**，`EngineHost` 由用户声明
//! （`none` / `UE4` / `UE5`）；`none` 就是"尚未确定"，处理方式与 PS 相同。

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// 画质档位。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QualityTier {
    #[default]
    Tier2,
    Tier1,
    Tier3,
    Tier4,
    Tier5,
}

impl QualityTier {
    pub fn from_text(value: &str) -> Result<Self> {
        match value.trim() {
            "Tier1" => Ok(Self::Tier1),
            "Tier2" => Ok(Self::Tier2),
            "Tier3" => Ok(Self::Tier3),
            "Tier4" => Ok(Self::Tier4),
            "Tier5" => Ok(Self::Tier5),
            _ => Err(Error::invalid("画质档位无效。")),
        }
    }

    pub fn as_text(self) -> &'static str {
        match self {
            Self::Tier1 => "Tier1",
            Self::Tier2 => "Tier2",
            Self::Tier3 => "Tier3",
            Self::Tier4 => "Tier4",
            Self::Tier5 => "Tier5",
        }
    }

    /// 分档目录名（与仓库里的实际目录一致）。
    pub fn folder(self) -> &'static str {
        match self {
            Self::Tier1 => "01-高性能均衡",
            Self::Tier2 => "02-主流均衡",
            Self::Tier3 => "03-性能优先",
            Self::Tier4 => "04-入门流畅",
            Self::Tier5 => "05-自定义",
        }
    }
}

/// 引擎宿主：由用户声明，不自动识别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EngineHost {
    #[default]
    None,
    Ue4,
    Ue5,
}

impl EngineHost {
    pub fn from_text(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "none" | "" => Ok(Self::None),
            "ue4" => Ok(Self::Ue4),
            "ue5" => Ok(Self::Ue5),
            _ => Err(Error::invalid("引擎校准设置无效。")),
        }
    }

    pub fn as_text(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Ue4 => "UE4",
            Self::Ue5 => "UE5",
        }
    }

    /// UE 主版本号；未声明时为 `None`。
    pub fn major(self) -> Option<u8> {
        match self {
            Self::None => None,
            Self::Ue4 => Some(4),
            Self::Ue5 => Some(5),
        }
    }
}

/// 解析出的画质配置源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualitySelection {
    pub host: EngineHost,
    pub tier: QualityTier,
    pub path: PathBuf,
    /// 结论来源，用于界面展示与日志。
    pub source: &'static str,
}

/// 按宿主与档位算出画质配置源。
pub fn resolve(host: EngineHost, tier: QualityTier, root: &Path) -> Result<QualitySelection> {
    let major = host
        .major()
        .ok_or_else(|| Error::invalid("尚未确定UE4/UE5，请等待识别或在高级设置校准引擎版本。"))?;
    let path = root
        .join("配置")
        .join("虚幻通用分类")
        .join(format!("UE{major}"))
        .join(tier.folder())
        .join("Engine.ini");
    Ok(QualitySelection {
        host,
        tier,
        path,
        source: "已声明 EngineHost",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_parsing_and_folders_follow_the_repository_layout() {
        assert_eq!(QualityTier::default(), QualityTier::Tier2);
        let expected = [
            (QualityTier::Tier1, "01-高性能均衡"),
            (QualityTier::Tier2, "02-主流均衡"),
            (QualityTier::Tier3, "03-性能优先"),
            (QualityTier::Tier4, "04-入门流畅"),
            (QualityTier::Tier5, "05-自定义"),
        ];
        for (tier, folder) in expected {
            assert_eq!(tier.folder(), folder);
            assert_eq!(QualityTier::from_text(tier.as_text()).expect("解析"), tier);
        }
    }

    #[test]
    fn invalid_tier_and_host_are_rejected() {
        for bad in ["Tier0", "Tier6", "tier2", "Current", ""] {
            let error = QualityTier::from_text(bad).expect_err(bad);
            assert!(
                error.to_string().contains("画质档位无效"),
                "{bad} → {error}"
            );
        }
        for bad in ["Auto", "UE3", "UE6"] {
            let error = EngineHost::from_text(bad).expect_err(bad);
            assert!(
                error.to_string().contains("引擎校准设置无效"),
                "{bad} → {error}"
            );
        }
        assert_eq!(
            EngineHost::from_text("none").expect("none"),
            EngineHost::None
        );
        assert_eq!(EngineHost::from_text("UE4").expect("UE4"), EngineHost::Ue4);
        assert_eq!(EngineHost::from_text("ue5").expect("ue5"), EngineHost::Ue5);
    }

    #[test]
    fn path_shape_matches_the_ps_layout() {
        let root = Path::new(r"D:\Games\dlssg_for_sm86\启动脚本");
        let selection = resolve(EngineHost::Ue5, QualityTier::Tier3, root).expect("解析");
        assert_eq!(
            selection.path,
            PathBuf::from(
                r"D:\Games\dlssg_for_sm86\启动脚本\配置\虚幻通用分类\UE5\03-性能优先\Engine.ini"
            )
        );
        let ue4 = resolve(EngineHost::Ue4, QualityTier::Tier5, root).expect("解析");
        assert_eq!(
            ue4.path,
            PathBuf::from(
                r"D:\Games\dlssg_for_sm86\启动脚本\配置\虚幻通用分类\UE4\05-自定义\Engine.ini"
            )
        );
    }

    #[test]
    fn undeclared_host_reports_the_same_wording_as_ps() {
        let error = resolve(EngineHost::None, QualityTier::Tier2, Path::new(r"D:\root"))
            .expect_err("应报错");
        assert!(
            error.to_string().contains("尚未确定UE4/UE5"),
            "实际：{error}"
        );
        assert_eq!(EngineHost::None.major(), None);
    }
}
