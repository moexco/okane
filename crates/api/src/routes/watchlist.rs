use axum::extract::{Path, State};

use crate::error::ApiError;
use crate::middleware::auth::CurrentUser;
use crate::server::AppState;
use crate::types::{ApiResponse, ApiResult, WatchlistRequest};

/// 获取自选股列表 (占位)
///
/// 获取当前用户的关注列表
#[utoipa::path(
    get,
    path = "/api/v1/user/watchlist",
    tag = "自选股 (Watchlist)",
    security(("bearer_jwt" = [])),
    responses(
        (status = 200, description = "获取成功", body = ApiResponse<Vec<String>>),
        (status = 500, description = "服务器内部错误")
    )
)]
pub async fn get_watchlist(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<ApiResult<Vec<String>>, ApiError> {
    match state.system_store.get_watchlist(&user.id).await {
        Ok(symbols) => Ok(ApiResult(symbols)),
        Err(e) => Err(ApiError::Internal(format!("store error: {}", e))),
    }
}

/// 添加自选股 (占位)
///
/// 将股票添加到自选
#[utoipa::path(
    post,
    path = "/api/v1/user/watchlist",
    tag = "自选股 (Watchlist)",
    security(("bearer_jwt" = [])),
    request_body = WatchlistRequest,
    responses(
        (status = 200, description = "添加成功", body = ApiResponse<String>),
        (status = 500, description = "服务器内部错误")
    )
)]
pub async fn add_to_watchlist(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    axum::Json(req): axum::Json<WatchlistRequest>,
) -> Result<ApiResult<String>, ApiError> {
    // 1. 验证目标证券是否存在
    // 首先看本地有没有
    let symbol_exists = match state.system_store.search_stocks(&req.symbol).await {
        Ok(results) => results.iter().any(|m| m.symbol == req.symbol),
        Err(_) => false,
    };

    // 如果本地没有，去上游查
    let is_valid = if symbol_exists {
        true
    } else {
        match state.market_port.search_symbols(&req.symbol).await {
            Ok(upstream) => upstream.iter().any(|m| m.symbol == req.symbol),
            Err(_) => false,
        }
    };

    if !is_valid {
        return Err(ApiError::BadRequest(
            "invalid symbol: stock not found".to_string(),
        ));
    }

    // 2. 插入自选股表
    match state
        .system_store
        .add_to_watchlist(&user.id, &req.symbol)
        .await
    {
        Ok(_) => Ok(ApiResult("ok".to_string())),
        Err(e) => Err(ApiError::Internal(format!("store error: {}", e))),
    }
}

/// 删除自选股 (占位)
///
/// 将股票从自选移除
#[utoipa::path(
    delete,
    path = "/api/v1/user/watchlist/{symbol}",
    tag = "自选股 (Watchlist)",
    security(("bearer_jwt" = [])),
    params(
        ("symbol" = String, Path, description = "股票代码")
    ),
    responses(
        (status = 200, description = "删除成功", body = ApiResponse<String>),
        (status = 500, description = "服务器内部错误")
    )
)]
pub async fn remove_from_watchlist(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(symbol): Path<String>,
) -> Result<ApiResult<String>, ApiError> {
    match state
        .system_store
        .remove_from_watchlist(&user.id, &symbol)
        .await
    {
        Ok(_) => Ok(ApiResult("ok".to_string())),
        Err(e) => Err(ApiError::Internal(format!("Store error: {}", e))),
    }
}
