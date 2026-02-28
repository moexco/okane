use axum::Json;
use axum::extract::Path;
use serde::Deserialize;
use utoipa::ToSchema;
use crate::types::ApiErrorResponse;
use crate::middleware::auth::CurrentUser;

/// 查询当前用户的活动订单 (占位)
/// 
/// # TODO
/// 查询挂单和历史充当
#[utoipa::path(
    get,
    path = "/api/v1/user/orders",
    tag = "订单交易 (Trade)",
    security(("bearer_jwt" = [])),
    responses(
        (status = 501, description = "功能未实现")
    )
)]
pub async fn get_orders(
    CurrentUser(_user): CurrentUser,
) -> Json<ApiErrorResponse> {
    Json(ApiErrorResponse::from_msg("501 Not Implemented"))
}

#[derive(Deserialize, ToSchema)]
pub struct PlaceOrderRequest {
    pub account_id: String,
    pub symbol: String,
    pub volume: f64,
    pub price: Option<f64>,
    pub order_type: String,
}

/// 提交新订单 (占位)
/// 
/// # TODO
/// 处理限价或市价报单逻辑
#[utoipa::path(
    post,
    path = "/api/v1/user/orders",
    tag = "订单交易 (Trade)",
    security(("bearer_jwt" = [])),
    request_body = PlaceOrderRequest,
    responses(
        (status = 501, description = "功能未实现")
    )
)]
pub async fn place_order(
    CurrentUser(_user): CurrentUser,
    Json(_req): Json<PlaceOrderRequest>,
) -> Json<ApiErrorResponse> {
    Json(ApiErrorResponse::from_msg("501 Not Implemented"))
}

/// 撤销订单 (占位)
/// 
/// # TODO
/// 将指定挂单取消
#[utoipa::path(
    delete,
    path = "/api/v1/user/orders/{order_id}",
    tag = "订单交易 (Trade)",
    security(("bearer_jwt" = [])),
    params(
        ("order_id" = String, Path, description = "系统订单 ID")
    ),
    responses(
        (status = 501, description = "功能未实现")
    )
)]
pub async fn cancel_order(
    CurrentUser(_user): CurrentUser,
    Path(_order_id): Path<String>,
) -> Json<ApiErrorResponse> {
    Json(ApiErrorResponse::from_msg("501 Not Implemented"))
}
