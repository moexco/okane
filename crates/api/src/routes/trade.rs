use axum::extract::{Path, Query, State};
use serde::Deserialize;
use utoipa::ToSchema;
use chrono::Utc;
use rust_decimal::Decimal;
use std::str::FromStr;

use okane_core::trade::entity::{AccountId, Order, OrderDirection, OrderId, AlgoOrder, AlgoType};
use crate::error::ApiError;
use crate::types::{ApiResponse, ApiResult, OrderResponse, AlgoOrderResponse, Page};
use crate::middleware::auth::CurrentUser;
use crate::server::AppState;

#[derive(Deserialize, ToSchema)]
pub struct GetOrdersQuery {
    pub account_id: String,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// 查询当前账户的活动订单
/// 
/// 查询挂单和活动单
#[utoipa::path(
    get,
    path = "/api/v1/user/orders",
    tag = "订单交易 (Trade)",
    security(("bearer_jwt" = [])),
    params(
        ("account_id" = String, Query, description = "系统账户 ID"),
        ("limit" = Option<usize>, Query, description = "返回数量限制，默认 50"),
        ("offset" = Option<usize>, Query, description = "跳过的记录数，默认 0")
    ),
    responses(
        (status = 200, description = "获取成功", body = ApiResponse<Page<OrderResponse>>),
        (status = 500, description = "服务器内部错误")
    )
)]
pub async fn get_orders(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(query): Query<GetOrdersQuery>,
) -> Result<ApiResult<Page<OrderResponse>>, ApiError> {
    let account = AccountId(query.account_id);
    
    // IDOR Check
    let is_owner = state.system_store.verify_account_ownership(&user.id, &account.0).await
        .map_err(|e| ApiError::Internal(format!("database error: {}", e)))?;
    if !is_owner {
        return Err(ApiError::Forbidden("forbidden: account does not belong to you".to_string()));
    }

    match state.trade_port.get_orders(&account).await {
        Ok(mut orders) => {
            // Sort by created_at descending (newest first)
            orders.sort_by_key(|b| std::cmp::Reverse(b.created_at));
            
            let total = orders.len();
            let offset = query.offset.unwrap_or(0);
            let limit = query.limit.unwrap_or(50).min(500);
            
            let paginated_orders: Vec<_> = orders.into_iter()
                .skip(offset)
                .take(limit)
                .collect();
                
            let items: Vec<OrderResponse> = paginated_orders.into_iter().map(Into::into).collect();
            
            // 使用标准的 Page 结构，包含在 data 字段内
            Ok(ApiResult(Page::new(items, total, offset, limit)))
        }
        Err(e) => Err(ApiError::Internal(format!("trade error: {}", e))),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct PlaceOrderRequest {
    pub account_id: String,
    pub symbol: String,
    pub volume: String,
    pub price: Option<String>,
    pub direction: String,
}

/// 提交新订单
/// 
/// 限价单将被挂载在交易队列中，市价单将与最新价直接撮合
#[utoipa::path(
    post,
    path = "/api/v1/user/orders",
    tag = "订单交易 (Trade)",
    security(("bearer_jwt" = [])),
    request_body = PlaceOrderRequest,
    responses(
        (status = 200, description = "提交成功，返回订单 ID", body = ApiResponse<String>),
        (status = 400, description = "参数验证错误"),
        (status = 500, description = "服务器内部错误")
    )
)]
pub async fn place_order(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    axum::Json(req): axum::Json<PlaceOrderRequest>,
) -> Result<ApiResult<String>, ApiError> {
    // IDOR Check
    let is_owner = state.system_store.verify_account_ownership(&user.id, &req.account_id).await
        .map_err(|e| ApiError::Internal(format!("database error: {}", e)))?;
    if !is_owner {
        return Err(ApiError::Forbidden("forbidden: account does not belong to you".to_string()));
    }

    let direction = match req.direction.to_uppercase().as_str() {
        "BUY" => OrderDirection::Buy,
        "SELL" => OrderDirection::Sell,
        _ => return Err(ApiError::BadRequest("invalid direction, expected buy or sell".to_string())),
    };
    
    let volume = Decimal::from_str(&req.volume)
        .map_err(|_| ApiError::BadRequest("invalid volume precision".to_string()))?;
    if volume <= Decimal::ZERO {
        return Err(ApiError::BadRequest("volume must be greater than zero".to_string()));
    }
        
    let price = match req.price {
        Some(p) => Some(Decimal::from_str(&p).map_err(|_| ApiError::BadRequest("invalid price precision".to_string()))?),
        None => None,
    };
    
    let order = Order::new(
        OrderId(uuid::Uuid::new_v4().to_string()),
        AccountId(req.account_id),
        req.symbol,
        direction,
        price,
        volume,
        Utc::now().timestamp_millis(),
    );

    match state.trade_port.submit_order(order).await {
        Ok(order_id) => Ok(ApiResult(order_id.0)),
        Err(e) => Err(ApiError::from(e)),
    }
}

/// 撤销订单
/// 
/// 仅能撤销处于 Pending 状态尚未全成交流水的队列订单
#[utoipa::path(
    delete,
    path = "/api/v1/user/orders/{order_id}",
    tag = "订单交易 (Trade)",
    security(("bearer_jwt" = [])),
    params(
        ("order_id" = String, Path, description = "系统订单 ID")
    ),
    responses(
        (status = 200, description = "撤销成功", body = ApiResponse<String>),
        (status = 404, description = "订单未找到或状态不可变更"),
        (status = 500, description = "服务器内部错误")
    )
)]
pub async fn cancel_order(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(order_id): Path<String>,
) -> Result<ApiResult<String>, ApiError> {
    let order_id = OrderId(order_id);
    
    // 1. 获取订单以验证归属权
    let order = state.trade_port.get_order(&order_id)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to fetch order: {}", e)))?
        .ok_or_else(|| ApiError::NotFound("order not found".to_string()))?;

    // 2. IDOR Check: Ensures the user owns the account associated with the order
    let is_owner = state.system_store.verify_account_ownership(&user.id, &order.account_id.0).await
        .map_err(|e| ApiError::Internal(format!("database error: {}", e)))?;
    if !is_owner {
        tracing::warn!("IDOR attempt: user {} tried to cancel order {} belonging to account {}", user.id, order.id.0, order.account_id.0);
        return Err(ApiError::Forbidden("forbidden: order does not belong to you".to_string()));
    }

    match state.trade_port.cancel_order(order_id).await {
        Ok(_) => Ok(ApiResult("ok".to_string())),
        Err(e) => Err(ApiError::from(e)),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct SubmitAlgoRequest {
    pub account_id: String,
    pub symbol: String,
    pub algo_type: String,
    pub params: serde_json::Value,
}

/// 提交算法单
#[utoipa::path(
    post,
    path = "/api/v1/user/algo",
    tag = "算法单 (Algo)",
    security(("bearer_jwt" = [])),
    request_body = SubmitAlgoRequest,
    responses(
        (status = 200, description = "提交成功", body = ApiResponse<String>),
        (status = 500, description = "内部错误")
    )
)]
pub async fn submit_algo_order(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    axum::Json(req): axum::Json<SubmitAlgoRequest>,
) -> Result<ApiResult<String>, ApiError> {
    let is_owner = state.system_store.verify_account_ownership(&user.id, &req.account_id).await
        .map_err(|e| ApiError::Internal(format!("database error: {}", e)))?;
    if !is_owner {
        return Err(ApiError::Forbidden("forbidden".to_string()));
    }

    let algo = match req.algo_type.as_str() {
        "grid" => {
            let upper = req.params["upper_price"].as_str().ok_or(ApiError::BadRequest("missing upper_price".into()))?;
            let lower = req.params["lower_price"].as_str().ok_or(ApiError::BadRequest("missing lower_price".into()))?;
            let grids = req.params["grids"].as_u64()
                .ok_or(ApiError::BadRequest("missing grids".into()))?
                .try_into()
                .map_err(|_| ApiError::BadRequest("grids value too large".into()))?;
            AlgoType::Grid {
                upper_price: Decimal::from_str(upper).map_err(|_| ApiError::BadRequest("invalid price".into()))?,
                lower_price: Decimal::from_str(lower).map_err(|_| ApiError::BadRequest("invalid price".into()))?,
                grids,
            }
        },
        "snipe" => {
            let target = req.params["target_price"].as_str().ok_or(ApiError::BadRequest("missing target_price".into()))?;
            AlgoType::Snipe {
                target_price: Decimal::from_str(target).map_err(|_| ApiError::BadRequest("invalid price".into()))?,
                max_slippage: Decimal::ZERO,
            }
        },
        _ => return Err(ApiError::BadRequest("unsupported algo type".into())),
    };

    let order = AlgoOrder::new(
        OrderId(uuid::Uuid::new_v4().to_string()),
        AccountId(req.account_id),
        req.symbol,
        algo,
        Utc::now().timestamp_millis(),
    );

    match state.algo_port.submit_algo_order(order).await {
        Ok(id) => Ok(ApiResult(id.0)),
        Err(e) => Err(ApiError::from(e)),
    }
}

/// 获取当前账户的所有算法单
#[utoipa::path(
    get,
    path = "/api/v1/user/algo",
    tag = "算法单 (Algo)",
    security(("bearer_jwt" = [])),
    params(
        ("account_id" = String, Query, description = "账户 ID")
    ),
    responses(
        (status = 200, description = "获取成功", body = ApiResponse<Vec<AlgoOrderResponse>>)
    )
)]
pub async fn get_algo_orders(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(query): Query<GetOrdersQuery>,
) -> Result<ApiResult<Vec<AlgoOrderResponse>>, ApiError> {
    let account = AccountId(query.account_id);
    let is_owner = state.system_store.verify_account_ownership(&user.id, &account.0).await
        .map_err(|e| ApiError::Internal(format!("database error: {}", e)))?;
    if !is_owner {
        return Err(ApiError::Forbidden("forbidden".to_string()));
    }

    match state.algo_port.get_algo_orders(&account).await {
        Ok(orders) => {
            let dtos = orders.into_iter().map(Into::into).collect();
            Ok(ApiResult(dtos))
        }
        Err(e) => Err(ApiError::from(e)),
    }
}

/// 撤销算法单
#[utoipa::path(
    delete,
    path = "/api/v1/user/algo/{algo_id}",
    tag = "算法单 (Algo)",
    security(("bearer_jwt" = [])),
    params(
        ("algo_id" = String, Path, description = "算法单 ID")
    ),
    responses(
        (status = 200, description = "撤销成功", body = ApiResponse<String>),
        (status = 404, description = "未找到算法单"),
        (status = 500, description = "内部错误")
    )
)]
pub async fn cancel_algo_order(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(algo_id): Path<String>,
) -> Result<ApiResult<String>, ApiError> {
    let order_id = OrderId(algo_id);
    
    // 权限检查
    let order = state.algo_port.get_algo_order(&order_id).await
        .map_err(|e| ApiError::Internal(format!("failed to fetch algo order: {}", e)))?
        .ok_or_else(|| ApiError::NotFound("algo order not found".to_string()))?;

    let is_owner = state.system_store.verify_account_ownership(&user.id, &order.account_id.0).await
        .map_err(|e| ApiError::Internal(format!("database error: {}", e)))?;
    if !is_owner {
        return Err(ApiError::Forbidden("forbidden".to_string()));
    }

    match state.algo_port.cancel_algo_order(&order_id).await {
        Ok(_) => Ok(ApiResult("ok".to_string())),
        Err(e) => Err(ApiError::from(e)),
    }
}

/// 查询账户持仓
#[utoipa::path(
    get,
    path = "/api/v1/user/account/{account_id}/positions",
    tag = "账户 (Account)",
    security(("bearer_jwt" = [])),
    params(
        ("account_id" = String, Path, description = "账户 ID")
    ),
    responses(
        (status = 200, description = "查询成功", body = ApiResponse<serde_json::Value>)
    )
)]
pub async fn get_positions(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(account_id): Path<String>,
) -> Result<ApiResult<serde_json::Value>, ApiError> {
    let account = AccountId(account_id);
    let is_owner = state.system_store.verify_account_ownership(&user.id, &account.0).await
        .map_err(|e| ApiError::Internal(format!("database error: {}", e)))?;
    if !is_owner {
        return Err(ApiError::Forbidden("forbidden".to_string()));
    }

    match state.trade_port.get_account(account).await {
        Ok(acc) => {
            // 返回持仓快照的 JSON 形式
            Ok(ApiResult(serde_json::to_value(acc.positions).map_err(|e| ApiError::Internal(e.to_string()))?))
        }
        Err(e) => Err(ApiError::from(e)),
    }
}
