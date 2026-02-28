use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// # Summary
/// 证券标的实体，代表系统关注的特定股票或资产。
///
/// # Invariants
/// - `symbol` 必须是合法的交易代码。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stock {
    // 股票代码 (例如: AAPL, 000001)
    pub symbol: String,
    // 交易所代码 (可选，例如: NASDAQ, SZ)
    pub exchange: Option<String>,
}

/// # Summary
/// 交易时间周期枚举，定义 K 线的时间跨度。
///
/// # Invariants
/// - 无特定约束。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum TimeFrame {
    // 1分钟
    Minute1,
    // 5分钟
    Minute5,
    // 1小时
    Hour1,
    // 1日
    Day1,
}

impl FromStr for TimeFrame {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "1m" | "minute1" => Ok(TimeFrame::Minute1),
            "5m" | "minute5" => Ok(TimeFrame::Minute5),
            "1h" | "hour1" => Ok(TimeFrame::Hour1),
            "1d" | "day1" => Ok(TimeFrame::Day1),
            _ => Err(format!("Unknown TimeFrame: {}", s)),
        }
    }
}

impl std::fmt::Display for TimeFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TimeFrame::Minute1 => write!(f, "1m"),
            TimeFrame::Minute5 => write!(f, "5m"),
            TimeFrame::Hour1 => write!(f, "1h"),
            TimeFrame::Day1 => write!(f, "1d"),
        }
    }
}
