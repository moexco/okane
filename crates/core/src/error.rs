use serde::{Deserialize, Serialize};
use thiserror::Error;

/// # Summary
/// Core 模块通用错误枚举。
#[derive(Error, Debug, Serialize, Deserialize)]
pub enum CoreError {
    #[error("lock poisoned: {0}")]
    Poisoned(String),

    #[error("internal error: {0}")]
    Internal(String),

    #[error("conversion error: {0}")]
    Conversion(String),
}
