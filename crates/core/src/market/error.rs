use thiserror::Error;
use serde::{Serialize, Deserialize};
use crate::error::CoreError;

/// # Summary
/// 市场数据域错误枚举，处理网络、解析及数据缺失等问题。
///
/// # Invariants
/// - 必须通过 `thiserror` 派生 `Error` trait。
#[derive(Error, Debug, Serialize, Deserialize)]
pub enum MarketError {
    // 网络层错误，包含底层 HTTP 客户端错误信息
    #[error("network error: {0}")]
    Network(String),
    // 数据解析错误，如 JSON 格式不匹配
    #[error("parse error: {0}")]
    Parse(String),
    // 请求的数据未找到 (404 或内容为空)
    #[error("data not found")]
    NotFound,
    // 内核底层错误（如锁污染）
    #[error("core error: {0}")]
    Core(#[from] CoreError),
    // 未知或未分类的错误
    #[error("unknown error: {0}")]
    Unknown(String),
}
