use crate::common::TimeFrame;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// # Summary
/// 运行在哪个策略引擎中。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EngineType {
    JavaScript,
    Wasm,
}

impl std::fmt::Display for EngineType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineType::JavaScript => write!(f, "JavaScript"),
            EngineType::Wasm => write!(f, "Wasm"),
        }
    }
}

impl std::str::FromStr for EngineType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "JavaScript" => Ok(EngineType::JavaScript),
            "Wasm" => Ok(EngineType::Wasm),
            _ => Err(format!("Unknown EngineType: {}", s)),
        }
    }
}

/// # Summary
/// 策略运行状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StrategyStatus {
    Pending,
    Running,
    Stopped,
    Failed(String), // 附带错误信息
}

impl std::fmt::Display for StrategyStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StrategyStatus::Pending => write!(f, "Pending"),
            StrategyStatus::Running => write!(f, "Running"),
            StrategyStatus::Stopped => write!(f, "Stopped"),
            StrategyStatus::Failed(msg) => write!(f, "Failed:{}", msg),
        }
    }
}

impl std::str::FromStr for StrategyStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Pending" => Ok(StrategyStatus::Pending),
            "Running" => Ok(StrategyStatus::Running),
            "Stopped" => Ok(StrategyStatus::Stopped),
            other if other.starts_with("Failed:") => {
                let msg = other.strip_prefix("Failed:").ok_or_else(|| "Invalid Failed prefix".to_string())?;
                Ok(StrategyStatus::Failed(msg.to_string()))
            }
            _ => Err(format!("Unknown StrategyStatus: {}", s)),
        }
    }
}

/// # Summary
/// `StrategyInstance` 聚合根。
///
/// # Invariants
/// - 代表系统内一个需要托管生命周期的策略单元。
/// - 与指定的用户和证券代码强绑定。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrategyInstance {
    pub id: String,
    pub symbol: String,
    pub account_id: String, // 绑定的交易账户
    pub timeframe: TimeFrame,
    pub engine_type: EngineType,
    pub source: Vec<u8>,
    pub status: StrategyStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
