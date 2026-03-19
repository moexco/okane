//! # API 统一错误处理
//!
//! 将下层各 crate 的错误类型统一映射到 HTTP 状态码与 JSON 响应体。

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub enum InternalErrorKind {
    Database,
    Upstream,
    Crypto,
    Serialization,
    Runtime,
}

impl InternalErrorKind {
    fn client_message(self) -> &'static str {
        match self {
            InternalErrorKind::Database => "database operation failed",
            InternalErrorKind::Upstream => "upstream service failed",
            InternalErrorKind::Crypto => "security operation failed",
            InternalErrorKind::Serialization => "serialization operation failed",
            InternalErrorKind::Runtime => "internal runtime failure",
        }
    }
}

/// API 层统一错误枚举
#[derive(Error, Debug)]
pub enum ApiError {
    // === 00: Common/System Errors (00XXX) ===
    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("internal server error")]
    Internal {
        kind: InternalErrorKind,
        fact: String,
    },

    // === 10: Auth & Identity Errors (10XXX) ===
    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("forbidden: {0}")]
    Forbidden(String),
}

impl ApiError {
    pub fn database(fact: impl Into<String>) -> Self {
        Self::Internal {
            kind: InternalErrorKind::Database,
            fact: fact.into(),
        }
    }

    pub fn upstream(fact: impl Into<String>) -> Self {
        Self::Internal {
            kind: InternalErrorKind::Upstream,
            fact: fact.into(),
        }
    }

    pub fn crypto(fact: impl Into<String>) -> Self {
        Self::Internal {
            kind: InternalErrorKind::Crypto,
            fact: fact.into(),
        }
    }

    pub fn serialization(fact: impl Into<String>) -> Self {
        Self::Internal {
            kind: InternalErrorKind::Serialization,
            fact: fact.into(),
        }
    }

    pub fn runtime(fact: impl Into<String>) -> Self {
        Self::Internal {
            kind: InternalErrorKind::Runtime,
            fact: fact.into(),
        }
    }

    /// 获取 HTTP 状态码和业务错误码 (MMXXX)
    pub fn codes(&self) -> (StatusCode, u32) {
        match self {
            // Common (00)
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, 400),
            ApiError::NotFound(_) => (StatusCode::NOT_FOUND, 404),
            ApiError::Internal { .. } => (StatusCode::INTERNAL_SERVER_ERROR, 500),

            // Auth (10)
            ApiError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, 10001),
            ApiError::Forbidden(_) => (StatusCode::FORBIDDEN, 10003),
        }
    }
}

/// 将 `ApiError` 转换为 axum 的 HTTP 响应
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, business_code) = self.codes();
        let message = match &self {
            ApiError::Internal { kind, fact } => {
                tracing::error!(
                    "internal server error [{}]: {}",
                    kind.client_message(),
                    fact
                );
                kind.client_message().to_string()
            }
            _ => self.to_string(),
        };

        let mut res = status.into_response();
        let marker = crate::types::ErrorMarker {
            message,
            status,
            code: business_code,
            latency_ms: None, // 彻底移除延迟逻辑，防止审计漏洞
        };
        res.extensions_mut().insert(std::sync::Arc::new(marker) as std::sync::Arc<dyn crate::types::ErasedResponse>);
        res
    }
}

/// 从 `ManagerError` 转换
impl From<okane_manager::strategy::ManagerError> for ApiError {
    fn from(err: okane_manager::strategy::ManagerError) -> Self {
        match &err {
            okane_manager::strategy::ManagerError::NotFound(msg) => ApiError::NotFound(msg.clone()),
            okane_manager::strategy::ManagerError::Store(
                okane_core::store::error::StoreError::NotFound,
            ) => ApiError::NotFound("not found".to_string()),
            okane_manager::strategy::ManagerError::AlreadyRunning(msg) => {
                ApiError::BadRequest(format!("strategy already running: {}", msg))
            }
            okane_manager::strategy::ManagerError::Store(store_err) => {
                ApiError::database(store_err.to_string())
            }
            _ => ApiError::runtime(err.to_string()),
        }
    }
}

/// 从 `TradeError` 转换
impl From<okane_core::trade::port::TradeError> for ApiError {
    fn from(err: okane_core::trade::port::TradeError) -> Self {
        match &err {
            okane_core::trade::port::TradeError::AccountNotFound(msg) => {
                ApiError::NotFound(msg.clone())
            }
            okane_core::trade::port::TradeError::InsufficientFunds { .. } => {
                ApiError::BadRequest(err.to_string())
            }
            okane_core::trade::port::TradeError::OrderNotFound(msg) => {
                ApiError::NotFound(msg.clone())
            }
            okane_core::trade::port::TradeError::BrokerIntegrationError(msg) => {
                ApiError::upstream(msg.clone())
            }
            _ => ApiError::runtime(err.to_string()),
        }
    }
}
