use thiserror::Error;

/// # Summary
/// 存储层错误枚举，处理数据库连接、读写失败等问题。
///
/// # Invariants
/// - 必须通过 `thiserror` 派生 `Error` trait。
#[derive(Error, Debug)]
pub enum StoreError {
    /// 数据库操作失败
    #[error("Database error: {0}")]
    Database(String),
    /// 记录未找到
    #[error("Not found")]
    NotFound,
    /// 未知或未分类的错误
    #[error("Unknown error: {0}")]
    Unknown(String),
    /// 初始化存储失败
    #[error("Initialization error: {0}")]
    InitError(String),
}
