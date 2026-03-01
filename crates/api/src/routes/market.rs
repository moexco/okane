use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use std::str::FromStr;
use chrono::{DateTime, Utc};

use okane_core::common::TimeFrame;
use crate::server::AppState;
use crate::types::{ApiErrorResponse, ApiResponse, CandleResponse, StockMetadataResponse};
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
pub struct SearchQuery {
    pub q: String,
}

/// 搜索股票
/// 
/// 根据关键字 (代码或名称) 从存储中模糊匹配搜索股票元数据。
#[utoipa::path(
    get,
    path = "/api/v1/market/search",
    tag = "行情 (Market)",
    params(
        ("q" = String, Query, description = "搜索关键字")
    ),
    responses(
        (status = 200, description = "搜索成功", body = ApiResponse<Vec<StockMetadataResponse>>),
        (status = 500, description = "内部服务器错误")
    )
)]
pub async fn search_stocks(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<ApiResponse<Vec<StockMetadataResponse>>>, Json<ApiErrorResponse>> {
    // 模糊搜索直接路由到上游数据源，不走本地数据库
    match state.market_port.search_symbols(&query.q).await {
        Ok(upstream_results) => {
            let dtos = upstream_results.into_iter().map(Into::into).collect();
            Ok(Json(ApiResponse::ok(dtos)))
        }
        Err(e) => Err(Json(ApiErrorResponse::from_msg(format!("Upstream search error: {}", e)))),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct CandlesQuery {
    pub tf: String,
    pub start: String,
    pub end: String,
}

/// 获取历史 K 线
/// 
/// 获取特定股票的多周期 K 线数据。时间必须为 RFC3339 格式 (例如: "2026-03-01T10:00:00Z")。
#[utoipa::path(
    get,
    path = "/api/v1/market/candles/{symbol}",
    tag = "行情 (Market)",
    params(
        ("symbol" = String, Path, description = "股票代码"),
        ("tf" = String, Query, description = "Timeframe (e.g., 1m, 1h, 1d)"),
        ("start" = String, Query, description = "ISO 8601 start time"),
        ("end" = String, Query, description = "ISO 8601 end time")
    ),
    responses(
        (status = 200, description = "拉取成功", body = ApiResponse<Vec<CandleResponse>>),
        (status = 400, description = "无效的请求参数"),
        (status = 500, description = "内部服务器错误")
    )
)]
pub async fn get_candles(
    State(state): State<AppState>,
    Path(symbol): Path<String>,
    Query(query): Query<CandlesQuery>,
) -> Result<Json<ApiResponse<Vec<CandleResponse>>>, Json<ApiErrorResponse>> {
    let tf = TimeFrame::from_str(&query.tf)
        .map_err(|e| Json(ApiErrorResponse::from_msg(format!("Invalid timeframe: {}", e))))?;
    
    let start = DateTime::parse_from_rfc3339(&query.start)
        .map_err(|_| Json(ApiErrorResponse::from_msg("Invalid start time format, expected RFC3339")))?
        .with_timezone(&Utc);
        
    let end = DateTime::parse_from_rfc3339(&query.end)
        .map_err(|_| Json(ApiErrorResponse::from_msg("Invalid end time format, expected RFC3339")))?
        .with_timezone(&Utc);

    let stock_agg = state.market_port.get_stock(&symbol).await
        .map_err(|e| Json(ApiErrorResponse::from_msg(format!("Market error: {}", e))))?;
        
    let history = stock_agg.fetch_history(tf, start, end).await
        .map_err(|e| Json(ApiErrorResponse::from_msg(format!("Fetch history error: {}", e))))?;
        
    let dtos = history.into_iter().map(Into::into).collect();
    Ok(Json(ApiResponse::ok(dtos)))
}
