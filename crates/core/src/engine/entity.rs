use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// # Summary
/// 策略产生的领域信号/事件。
///
/// # Invariants
/// - 包含完整的上下文信息，以支持多目标分发逻辑。
/// - 必须支持序列化，以便跨越 WASM 边界。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signal {
    // 信号唯一标识
    pub id: String,
    // 证券身份 (e.g., AAPL)
    pub symbol: String,
    // 产生时间
    pub timestamp: DateTime<Utc>,
    // 信号种类
    pub kind: SignalKind,
    // 产生信号的策略 ID
    pub strategy_id: String,
    // 附加元数据
    pub metadata: HashMap<String, String>,
}

/// # Summary
/// 信号的业务分类。
///
/// # Invariants
/// - 区分交易动作与纯信息通知。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SignalKind {
    // 进场做多
    LongEntry,
    // 进场做空
    ShortEntry,
    // 多头平仓
    LongExit,
    // 空头平仓
    ShortExit,
    // 纯信息警告
    Alert,
    // 纯状态信息
    Info,
}
