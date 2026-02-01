use thiserror::Error;

/// # Summary
/// 市场数据域错误枚举，处理网络、解析及数据缺失等问题。
///
/// # Invariants
/// - 必须通过 `thiserror` 派生 `Error` trait。
#[derive(Error, Debug)]
pub enum MarketError {
    // 网络层错误，包含底层 HTTP 客户端错误信息
    #[error("Network error: {0}")]
    Network(String),
    // 数据解析错误，如 JSON 格式不匹配
    #[error("Parse error: {0}")]
    Parse(String),
    // 请求的数据未找到 (404 或内容为空)
    #[error("Data not found")]
    NotFound,
    // 未知或未分类的错误
    #[error("Unknown error: {0}")]
    Unknown(String),
}
