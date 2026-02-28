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
    pub timeframe: TimeFrame,
    pub engine_type: EngineType,
    pub source: Vec<u8>,
    pub status: StrategyStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
