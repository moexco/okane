use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use utoipa::ToSchema;
use chrono::Utc;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;

use okane_core::trade::entity::{AccountId, Order, OrderDirection, OrderId};
use crate::types::{ApiErrorResponse, ApiResponse, OrderResponse};
use crate::middleware::auth::CurrentUser;
use crate::server::AppState;

#[derive(Deserialize, ToSchema)]
pub struct GetOrdersQuery {
    pub account_id: String,
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
        ("account_id" = String, Query, description = "系统账户 ID")
    ),
    responses(
        (status = 200, description = "获取成功", body = ApiResponse<Vec<OrderResponse>>),
        (status = 500, description = "服务器内部错误")
    )
)]
pub async fn get_orders(
    State(state): State<AppState>,
    CurrentUser(_user): CurrentUser,
    Query(query): Query<GetOrdersQuery>,
) -> Result<Json<ApiResponse<Vec<OrderResponse>>>, Json<ApiErrorResponse>> {
    let account = AccountId(query.account_id);
    match state.trade_port.get_orders(&account).await {
        Ok(orders) => {
            let dtos = orders.into_iter().map(Into::into).collect();
            Ok(Json(ApiResponse::ok(dtos)))
        }
        Err(e) => Err(Json(ApiErrorResponse::from_msg(format!("Trade error: {}", e)))),
    }
}

#[derive(Deserialize, ToSchema)]
pub struct PlaceOrderRequest {
    pub account_id: String,
    pub symbol: String,
    pub volume: f64,
    pub price: Option<f64>,
    pub order_type: String,
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
    CurrentUser(_user): CurrentUser,
    Json(req): Json<PlaceOrderRequest>,
) -> Result<Json<ApiResponse<String>>, Json<ApiErrorResponse>> {
    let direction = match req.order_type.to_uppercase().as_str() {
        "BUY" => OrderDirection::Buy,
        "SELL" => OrderDirection::Sell,
        _ => return Err(Json(ApiErrorResponse::from_msg("Invalid order_type, expected Buy or Sell"))),
    };
    
    let volume = Decimal::from_f64(req.volume)
        .ok_or_else(|| Json(ApiErrorResponse::from_msg("Invalid volume precision")))?;
        
    let price = match req.price {
        Some(p) => Some(Decimal::from_f64(p).ok_or_else(|| Json(ApiErrorResponse::from_msg("Invalid price precision")))?),
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
    CurrentUser(_user): CurrentUser,
    Path(order_id): Path<String>,
) -> Result<Json<ApiResponse<String>>, Json<ApiErrorResponse>> {
    match state.trade_port.cancel_order(OrderId(order_id)).await {
        Ok(_) => Ok(Json(ApiResponse::ok("ok".to_string()))),
        Err(e) => Err(Json(ApiErrorResponse::from_msg(format!("Failed to cancel order: {}", e)))),
    }
}
