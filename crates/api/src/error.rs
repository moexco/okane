//! # API 统一错误处理
//!
//! 将下层各 crate 的错误类型统一映射到 HTTP 状态码与 JSON 响应体。

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use thiserror::Error;

use crate::types::ApiErrorResponse;

/// API 层统一错误枚举
#[derive(Error, Debug)]
pub enum ApiError {
    /// 认证失败 (401)
    #[error("认证失败: {0}")]
    Unauthorized(String),

    /// 权限不足 (403)
    #[error("权限不足: {0}")]
    Forbidden(String),

    /// 资源未找到 (404)
    #[error("资源未找到: {0}")]
    NotFound(String),

    /// 请求参数错误 (400)
    #[error("请求参数错误: {0}")]
    BadRequest(String),

    /// 下层业务错误 (500)
    #[error("内部服务错误: {0}")]
    Internal(String),
}

/// 将 `ApiError` 转换为 axum 的 HTTP 响应
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            ApiError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            ApiError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg.clone()),
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            ApiError::Internal(msg) => {
                // 内部错误只记录日志，不向客户端透传细节
                tracing::error!("内部服务错误: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "服务器内部错误".to_string(),
                )
            }
        };

        let body = Json(ApiErrorResponse::from_msg(message));
        (status, body).into_response()
    }
}

/// 从 `ManagerError` 转换
impl From<okane_manager::strategy::ManagerError> for ApiError {
    fn from(err: okane_manager::strategy::ManagerError) -> Self {
        match &err {
            okane_manager::strategy::ManagerError::NotFound(msg) => {
                ApiError::NotFound(msg.clone())
            }
            _ => ApiError::Internal(err.to_string()),
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
            _ => ApiError::Internal(err.to_string()),
        }
    }
}
