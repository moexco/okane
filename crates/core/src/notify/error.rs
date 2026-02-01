use thiserror::Error;

/// # Summary
/// 通知服务错误枚举。
///
/// # Invariants
/// - 必须通过 `thiserror` 派生 `Error` trait。
#[derive(Error, Debug)]
pub enum NotifyError {
    /// 网络连接或传输错误
    #[error("Network error: {0}")]
    Network(String),

    /// 配置错误 (如缺少 Token)
    #[error("Configuration error: {0}")]
    Config(String),

    /// 推送平台返回的错误 (如 Telegram API Error)
    #[error("Platform error: {0}")]
    Platform(String),
}
