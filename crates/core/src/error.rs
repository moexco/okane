use thiserror::Error;

/// # Summary
/// Core 模块通用错误枚举。
#[derive(Error, Debug)]
pub enum CoreError {
    #[error("Lock poisoned: {0}")]
    Poisoned(String),
    
    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Conversion error: {0}")]
    Conversion(String),
}
