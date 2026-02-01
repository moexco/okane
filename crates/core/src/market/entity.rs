use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// # Summary
/// 单根 K 线数据实体，记录特定时段内的行情波动。
///
/// # Invariants
/// - `high` 必须大于或等于 `low`, `open`, `close`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candle {
    // K 线开始时间
    pub time: DateTime<Utc>,
    // 开盘价
    pub open: f64,
    // 最高价
    pub high: f64,
    // 最低价
    pub low: f64,
    // 收盘价
    pub close: f64,
    // 调整后收盘价 (用于处理分红、拆股等复权情况)
    pub adj_close: Option<f64>,
    // 成交量
    pub volume: f64,
    // 是否为最终数据 (即该周期已收盘)
    pub is_final: bool,
}
