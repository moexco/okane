use axum::extract::{Path, Query, State, ws::{WebSocketUpgrade, WebSocket, Message}};
use serde::Deserialize;
use std::str::FromStr;
use chrono::{DateTime, Utc};
use futures::StreamExt;

use okane_core::common::TimeFrame;
use crate::server::AppState;
use crate::error::ApiError;
use crate::types::{ApiResponse, ApiResult, CandleResponse, StockMetadataResponse};
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
    security(("bearer_jwt" = [])),
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
) -> Result<ApiResult<Vec<StockMetadataResponse>>, ApiError> {
    // 模糊搜索直接路由到上游数据源，不走本地数据库
    match state.market_port.search_symbols(&query.q).await {
        Ok(upstream_results) => {
            let dtos = upstream_results.into_iter().map(Into::into).collect();
            Ok(ApiResult(dtos))
        }
        Err(e) => Err(ApiError::Internal(format!("upstream search error: {}", e))),
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
    security(("bearer_jwt" = [])),
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
) -> Result<ApiResult<Vec<CandleResponse>>, ApiError> {
    let tf = TimeFrame::from_str(&query.tf)
        .map_err(|e| ApiError::BadRequest(format!("invalid timeframe: {}", e)))?;
    
    let start = DateTime::parse_from_rfc3339(&query.start)
        .map_err(|_| ApiError::BadRequest("invalid start time format, expected RFC3339".to_string()))?
        .with_timezone(&Utc);
        
    let end = DateTime::parse_from_rfc3339(&query.end)
        .map_err(|_| ApiError::BadRequest("invalid end time format, expected RFC3339".to_string()))?
        .with_timezone(&Utc);

    let stock_agg = state.market_port.get_stock(&symbol).await
        .map_err(|e| ApiError::Internal(format!("market error: {}", e)))?;
        
    let history: Vec<okane_core::market::entity::Candle> = stock_agg.fetch_history(tf, start, end).await
        .map_err(|e| ApiError::Internal(format!("fetch history error: {}", e)))?;
        
    let dtos = history.into_iter().map(Into::into).collect();
    Ok(ApiResult(dtos))
}

#[derive(Deserialize, ToSchema)]
pub struct IndicatorQuery {
    pub tf: String,
    pub period: u32,
}

/// 获取 RSI 指标
#[utoipa::path(
    get,
    path = "/api/v1/market/indicator/rsi/{symbol}",
    tag = "行情 (Market)",
    security(("bearer_jwt" = [])),
    params(
        ("symbol" = String, Path, description = "股票代码"),
        ("tf" = String, Query, description = "Timeframe"),
        ("period" = u32, Query, description = "RSI 周期")
    ),
    responses(
        (status = 200, description = "获取成功", body = ApiResponse<String>)
    )
)]
pub async fn get_rsi_indicator(
    State(state): State<AppState>,
    Path(symbol): Path<String>,
    Query(query): Query<IndicatorQuery>,
) -> Result<ApiResult<String>, ApiError> {
    let tf = TimeFrame::from_str(&query.tf)
        .map_err(|e| ApiError::BadRequest(format!("invalid timeframe: {}", e)))?;

    match state.indicator_service.rsi(&symbol, tf, query.period).await {
        Ok(val) => Ok(ApiResult(val.to_string())),
        Err(e) => Err(ApiError::Internal(format!("indicator error: {}", e))),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct MarketWsParams {
    pub tf: String,
}

/// 行情实时推送 (WebSocket)
/// 
/// 建立 WebSocket 连接以接收特定股票的实时 K 线推送。
/// 参数: tf (TimeFrame, 例如 1m)
#[utoipa::path(
    get,
    path = "/api/v1/market/ws/{symbol}",
    tag = "行情 (Market)",
    security(("bearer_jwt" = [])),
    params(
        ("symbol" = String, Path, description = "股票代码"),
        ("tf" = String, Query, description = "Timeframe (e.g., 1m, 1h, 1d)")
    ),
    responses(
        (status = 101, description = "切换协议成功，开始推送实时 K 线")
    )
)]
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Path(symbol): Path<String>,
    Query(query): Query<MarketWsParams>,
) -> impl axum::response::IntoResponse {
    tracing::info!("WebSocket upgrade request for symbol: {}, tf: {}", symbol, query.tf);
    ws.on_upgrade(move |socket| handle_socket(socket, state, symbol, query))
}

async fn handle_socket(mut socket: WebSocket, state: AppState, symbol: String, query: MarketWsParams) {
    let tf = match TimeFrame::from_str(&query.tf) {
        Ok(t) => t,
        Err(_) => return,
    };

    let stock_agg = match state.market_port.get_stock(&symbol).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("WS: Failed to get stock {}: {}", symbol, e);
            return;
        }
    };

    let mut stream = match stock_agg.subscribe(tf) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("WS: Failed to subscribe to {}: {}", symbol, e);
            return;
        }
    };

    tracing::info!("WS client connected for {} [{:?}]", symbol, tf);

    loop {
        tokio::select! {
            Some(result) = stream.next() => {
                match result {
                    Ok(candle) => {
                        let msg = match serde_json::to_string(&candle) {
                            Ok(json) => Message::Text(json.into()),
                            Err(e) => {
                                tracing::error!("WS: Serialization error: {}", e);
                                continue;
                            }
                        };
                        if let Err(e) = socket.send(msg).await {
                            tracing::debug!("WS: Client disconnected from {}: {}", symbol, e);
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::error!("WS: Stream error for {}: {}", symbol, e);
                    }
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {} // 忽略其他消息（如 ping/pong 由 axum 自动处理）
                }
            }
        }
    }
    tracing::info!("WS client disconnected for {} [{:?}]", symbol, tf);
}
