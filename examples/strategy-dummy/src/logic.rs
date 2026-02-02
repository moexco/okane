use chrono::{DateTime, Utc};
use extism_pdk::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// # Summary
/// 策略输入数据结构 (K 线)。
///
/// # Invariants
/// - 与核心域 `Candle` 结构保持序列化兼容。
#[derive(Debug, Deserialize)]
pub struct Candle {
    // 时间戳
    pub time: DateTime<Utc>,
    // 开盘价
    pub open: f64,
    // 最高价
    pub high: f64,
    // 最低价
    pub low: f64,
    // 收盘价
    pub close: f64,
    // 成交量
    pub volume: f64,
    // 是否闭合
    pub is_final: bool,
}

/// # Summary
/// 策略输出信号结构。
///
/// # Invariants
/// - 与核心域 `Signal` 结构保持序列化兼容。
#[derive(Debug, Serialize)]
pub struct Signal {
    // 唯一标识
    pub id: String,
    // 证券代码
    pub symbol: String,
    // 产生时间
    pub timestamp: DateTime<Utc>,
    // 信号种类
    pub kind: SignalKind,
    // 策略 ID
    pub strategy_id: String,
    // 附加信息
    pub metadata: HashMap<String, String>,
}

/// # Summary
/// 信号种类枚举。
///
/// # Invariants
/// - 必须涵盖交易指令与系统通知。
#[derive(Debug, Serialize, PartialEq)]
pub enum SignalKind {
    // 进场做多
    LongEntry,
    // 进场做空
    ShortEntry,
    // 多头平仓
    LongExit,
    // 空头平仓
    ShortExit,
    // 警告
    Alert,
    // 信息
    Info,
}

/// # Summary
/// 策略逻辑入口。
///
/// # Logic
/// 1. 解析输入的 JSON 字符串为 `Candle` 对象。
/// 2. 判断收盘价是否大于 150.0。
/// 3. 若满足条件，构造并返回一个做多信号。
/// 4. 否则返回 null 字符串。
///
/// # Arguments
/// * `input` - 包含 K 线数据的 JSON 字符串。
///
/// # Returns
/// * `FnResult<String>` - 包含 `Option<Signal>` 的 JSON 字符串结果。
#[plugin_fn]
pub fn on_candle(input: String) -> FnResult<String> {
    let candle: Candle = serde_json::from_str(&input).map_err(WithReturnCode::from)?;

    if candle.close > 150.0 {
        let signal = Signal {
            id: "sig_dummy_001".to_string(),
            symbol: "DUMMY".to_string(),
            timestamp: Utc::now(),
            kind: SignalKind::LongEntry,
            strategy_id: "dummy-strategy".to_string(),
            metadata: HashMap::new(),
        };
        let output = serde_json::to_string(&Some(signal)).map_err(WithReturnCode::from)?;
        Ok(output)
    } else {
        Ok("null".to_string())
    }
}
