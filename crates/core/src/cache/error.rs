use thiserror::Error;

/// # Summary
/// 缓存域错误枚举，处理序列化、并发冲突及底层存储故障。
///
/// # Invariants
/// - 必须通过 `thiserror` 派生 `Error` trait。
#[derive(Error, Debug)]
pub enum CacheError {
    // 数据序列化失败
    #[error("Serialize error: {0}")]
    Serialize(String),
    // 数据反序列化失败
    #[error("Deserialize error: {0}")]
    Deserialize(String),
    // 底层存储引擎故障
    #[error("Storage error: {0}")]
    Storage(String),
    // 未知或未分类的错误
    #[error("Unknown error: {0}")]
    Unknown(String),
}
