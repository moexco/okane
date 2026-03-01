use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use utoipa::ToSchema;
use chrono::Utc;
use rust_decimal::Decimal;
use std::str::FromStr;

use okane_core::trade::entity::{AccountId, Order, OrderDirection, OrderId};
use crate::types::{ApiErrorResponse, ApiResponse, OrderResponse};
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
        (status = 200, description = "获取成功", body = ApiResponse<Vec<OrderResponse>>),
        (status = 500, description = "服务器内部错误")
    )
)]
pub async fn get_orders(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(query): Query<GetOrdersQuery>,
) -> Result<Json<ApiResponse<Vec<OrderResponse>>>, Json<ApiErrorResponse>> {
    let account = AccountId(query.account_id);
    
    // IDOR Check
    if account.0 != user.id && !account.0.starts_with(&format!("{}_", user.id)) {
        return Err(Json(ApiErrorResponse::from_msg("Forbidden: Account does not belong to you")));
    }

    match state.trade_port.get_orders(&account).await {
        Ok(mut orders) => {
            // Sort by created_at descending (newest first)
            orders.sort_by_key(|b| std::cmp::Reverse(b.created_at));
            
            let offset = query.offset.unwrap_or(0);
            let limit = query.limit.unwrap_or(50);
            
            let paginated_orders: Vec<_> = orders.into_iter()
                .skip(offset)
                .take(limit)
                .collect();
                
            let dtos = paginated_orders.into_iter().map(Into::into).collect();
            Ok(Json(ApiResponse::ok(dtos)))
        }
        Err(e) => Err(Json(ApiErrorResponse::from_msg(format!("Trade error: {}", e)))),
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
    Json(req): Json<PlaceOrderRequest>,
) -> Result<Json<ApiResponse<String>>, Json<ApiErrorResponse>> {
    // IDOR Check
    if req.account_id != user.id && !req.account_id.starts_with(&format!("{}_", user.id)) {
        return Err(Json(ApiErrorResponse::from_msg("Forbidden: Account does not belong to you")));
    }

    let direction = match req.direction.to_uppercase().as_str() {
        "BUY" => OrderDirection::Buy,
        "SELL" => OrderDirection::Sell,
        _ => return Err(Json(ApiErrorResponse::from_msg("Invalid direction, expected Buy or Sell"))),
    };
    
    let volume = Decimal::from_str(&req.volume)
        .map_err(|_| Json(ApiErrorResponse::from_msg("Invalid volume precision")))?;
    if volume <= Decimal::ZERO {
        return Err(Json(ApiErrorResponse::from_msg("Volume must be greater than zero")));
    }
        
    let price = match req.price {
        Some(p) => Some(Decimal::from_str(&p).map_err(|_| Json(ApiErrorResponse::from_msg("Invalid price precision")))?),
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
        Ok(order_id) => Ok(Json(ApiResponse::ok(order_id.0))),
        Err(e) => Err(Json(ApiErrorResponse::from_msg(format!("Trade execution error: {}", e)))),
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
) -> Result<Json<ApiResponse<String>>, Json<ApiErrorResponse>> {
    let order_id = OrderId(order_id);
    
    // 1. 获取订单以验证归属权
    let order = state.trade_port.get_order(&order_id)
        .await
        .map_err(|e| Json(ApiErrorResponse::from_msg(format!("Failed to fetch order: {}", e))))?
        .ok_or_else(|| Json(ApiErrorResponse::from_msg("Order not found".to_string())))?;

    // 2. IDOR Check: Ensures the user owns the account associated with the order
    if order.account_id.0 != user.id && !order.account_id.0.starts_with(&format!("{}_", user.id)) {
        tracing::warn!("IDOR attempt: user {} tried to cancel order {} belonging to account {}", user.id, order.id.0, order.account_id.0);
        return Err(Json(ApiErrorResponse::from_msg("Forbidden: Order does not belong to you")));
    }

    match state.trade_port.cancel_order(order_id).await {
        Ok(_) => Ok(Json(ApiResponse::ok("ok".to_string()))),
        Err(e) => Err(Json(ApiErrorResponse::from_msg(format!("Failed to cancel order: {}", e)))),
    }
}
