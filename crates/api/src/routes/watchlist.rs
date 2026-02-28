use axum::Json;
use axum::extract::Path;
use serde::Deserialize;
use utoipa::ToSchema;
use crate::types::ApiErrorResponse;
use crate::middleware::auth::CurrentUser;

/// 获取自选股列表 (占位)
/// 
/// # TODO
/// 获取当前用户的关注列表
#[utoipa::path(
    get,
    path = "/api/v1/user/watchlist",
    tag = "自选股 (Watchlist)",
    security(("bearer_jwt" = [])),
    responses(
        (status = 501, description = "功能未实现")
    )
)]
pub async fn get_watchlist(
    CurrentUser(_user): CurrentUser,
) -> Json<ApiErrorResponse> {
    Json(ApiErrorResponse::from_msg("501 Not Implemented"))
}

#[derive(Deserialize, ToSchema)]
pub struct WatchlistRequest {
    pub symbol: String,
}

/// 添加自选股 (占位)
/// 
/// # TODO
/// 将股票添加到自选
#[utoipa::path(
    post,
    path = "/api/v1/user/watchlist",
    tag = "自选股 (Watchlist)",
    security(("bearer_jwt" = [])),
    request_body = WatchlistRequest,
    responses(
        (status = 501, description = "功能未实现")
    )
)]
pub async fn add_to_watchlist(
    CurrentUser(_user): CurrentUser,
    Json(_req): Json<WatchlistRequest>,
) -> Json<ApiErrorResponse> {
    Json(ApiErrorResponse::from_msg("501 Not Implemented"))
}

/// 删除自选股 (占位)
/// 
/// # TODO
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
        (status = 501, description = "功能未实现")
    )
)]
pub async fn remove_from_watchlist(
    CurrentUser(_user): CurrentUser,
    Path(_symbol): Path<String>,
) -> Json<ApiErrorResponse> {
    Json(ApiErrorResponse::from_msg("501 Not Implemented"))
}
