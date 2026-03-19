use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// # Summary
/// 运行在哪个策略引擎中。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub enum EngineType {
    JavaScript,
}

impl std::fmt::Display for EngineType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineType::JavaScript => write!(f, "JavaScript"),
        }
    }
}

impl std::str::FromStr for EngineType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "JavaScript" => Ok(EngineType::JavaScript),
            _ => Err(format!("Unknown EngineType: {}", s)),
        }
    }
}

/// # Summary
/// 策略运行状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub enum StrategyStatus {
    Pending,
    Running,
    Stopped,
    Failed(String), // 附带错误信息
}

/// # Summary
/// 策略运行模式。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub enum StrategyRunMode {
    Backtest,
    LivePaper,
    LiveSignal,
    AutoTrade,
}

impl std::fmt::Display for StrategyRunMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StrategyRunMode::Backtest => write!(f, "Backtest"),
            StrategyRunMode::LivePaper => write!(f, "LivePaper"),
            StrategyRunMode::LiveSignal => write!(f, "LiveSignal"),
            StrategyRunMode::AutoTrade => write!(f, "AutoTrade"),
        }
    }
}

impl std::str::FromStr for StrategyRunMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Backtest" => Ok(StrategyRunMode::Backtest),
            "LivePaper" => Ok(StrategyRunMode::LivePaper),
            "LiveSignal" => Ok(StrategyRunMode::LiveSignal),
            "AutoTrade" => Ok(StrategyRunMode::AutoTrade),
            _ => Err(format!("Unknown StrategyRunMode: {}", s)),
        }
    }
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
                let msg = other
                    .strip_prefix("Failed:")
                    .ok_or_else(|| "Invalid Failed prefix".to_string())?;
                Ok(StrategyStatus::Failed(msg.to_string()))
            }
            _ => Err(format!("Unknown StrategyStatus: {}", s)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
    Debug,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::Info => write!(f, "INFO"),
            LogLevel::Warn => write!(f, "WARN"),
            LogLevel::Error => write!(f, "ERROR"),
            LogLevel::Debug => write!(f, "DEBUG"),
        }
    }
}

/// # Summary
/// 策略运行生成的日志条目。
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct StrategyLogEntry {
    pub strategy_id: String,
    pub level: LogLevel,
    pub message: String,
    pub timestamp: DateTime<Utc>,
}

/// # Summary
/// `StrategyInstance` 聚合根。
///
/// # Invariants
/// - 代表系统内一个可持续编辑与运行的策略实体。
/// - 承载草稿源码、默认运行输入与最新运行状态快照。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct StrategyInstance {
    pub id: String,
    pub name: String,
    pub symbol: String,
    pub account_id: String,
    pub timeframe: crate::common::TimeFrame,
    pub engine_type: EngineType,
    #[schema(value_type = String, format = "binary")]
    pub source: Vec<u8>,
    #[schema(value_type = Object)]
    pub parameter_schema: serde_json::Value,
    pub latest_run_id: Option<String>,
    pub status: StrategyStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// # Summary
/// 策略运行结果记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct StrategyRunRecord {
    pub id: String,
    pub strategy_id: String,
    pub symbol: String,
    pub account_id: String,
    pub timeframe: crate::common::TimeFrame,
    pub engine_type: EngineType,
    pub mode: StrategyRunMode,
    #[schema(value_type = String, format = "binary")]
    pub source: Vec<u8>,
    #[schema(value_type = Object)]
    pub parameter_values: serde_json::Value,
    #[schema(value_type = Object)]
    pub summary: serde_json::Value,
    pub status: StrategyStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
