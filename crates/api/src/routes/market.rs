use axum::Json;
use axum::extract::{Path, Query};
use serde::Deserialize;
use crate::types::ApiErrorResponse;
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
pub struct SearchQuery {
    pub q: String,
}

/// 搜索股票 (占位)
/// 
/// # TODO
/// 当前未实现，需通过 MarketStore 进行实际查询。
#[utoipa::path(
    get,
    path = "/api/v1/market/search",
    tag = "行情 (Market)",
    params(
        ("q" = String, Query, description = "搜索关键字")
    ),
    responses(
        (status = 501, description = "功能未实现")
    )
)]
pub async fn search_stocks(
    Query(_query): Query<SearchQuery>,
) -> Json<ApiErrorResponse> {
    Json(ApiErrorResponse::from_msg("501 Not Implemented: search_stocks is not yet implemented"))
}

#[derive(Deserialize, ToSchema)]
pub struct CandlesQuery {
    pub tf: String,
    pub start: String,
    pub end: String,
}

/// 获取历史 K 线 (占位)
/// 
/// # TODO
/// 当前未实现，需通过 MarketStore 读取历史 K 线数据。
#[utoipa::path(
    get,
    path = "/api/v1/market/candles/{symbol}",
    tag = "行情 (Market)",
    params(
        ("symbol" = String, Path, description = "股票代码"),
        ("tf" = String, Query, description = "Timeframe (e.g., 1m, 1d)"),
        ("start" = String, Query, description = "ISO 8601 start time"),
        ("end" = String, Query, description = "ISO 8601 end time")
    ),
    responses(
        (status = 501, description = "功能未实现")
    )
)]
pub async fn get_candles(
    Path(_symbol): Path<String>,
    Query(_query): Query<CandlesQuery>,
) -> Json<ApiErrorResponse> {
    Json(ApiErrorResponse::from_msg("501 Not Implemented: get_candles is not yet implemented"))
}
