//! # 策略管理路由控制器
//!
//! 实现 `/api/v1/user/strategies` 路径下的 REST 接口。
//! 对应 UI 原型中的策略列表、部署/停止、以及 Strategy Lab 的代码保存逻辑。

use axum::extract::{Path, Query, State};
use serde::Deserialize;
use axum::Json;

use crate::types::{ApiResponse, StartStrategyRequest, StrategyResponse};
use crate::error::ApiError;
use crate::middleware::auth::CurrentUser;
use crate::server::AppState;

// ============================================================
//  Handler 实现
// ============================================================

#[derive(Deserialize, utoipa::ToSchema)]
pub struct ListStrategiesQuery {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// 列出当前用户的所有策略实例
///
/// 返回该用户名下的全量策略列表及其运行状态。
/// 对应 UI 原型中 Strategy 页面所需的数据卡片。
#[utoipa::path(
    get,
    path = "/api/v1/user/strategies",
    tag = "策略 (Strategy)",
    security(("bearer_jwt" = [])),
    params(
        ("limit" = Option<usize>, Query, description = "返回数量限制，默认 50"),
        ("offset" = Option<usize>, Query, description = "跳过的记录数，默认 0")
    ),
    responses(
        (status = 200, description = "策略列表获取成功", body = ApiResponse<Vec<StrategyResponse>>),
        (status = 401, description = "未认证")
    )
)]
pub async fn list_strategies(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(query): Query<ListStrategiesQuery>,
) -> Result<Json<ApiResponse<Vec<StrategyResponse>>>, ApiError> {
    let mut instances = state.strategy_manager.list_strategies(&user.id).await?;
    
    // Sort by created_at descending (newest first)
    instances.sort_by_key(|b| std::cmp::Reverse(b.created_at));
    
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(50);
    
    let paginated_instances: Vec<_> = instances.into_iter()
        .skip(offset)
        .take(limit)
        .collect();

    let responses: Vec<StrategyResponse> = paginated_instances.iter().map(StrategyResponse::from).collect();

    Ok(Json(ApiResponse::ok(responses)))
}

/// 获取指定策略实例的详情
///
/// 通过策略 ID 拉取单个策略的完整信息。
#[utoipa::path(
    get,
    path = "/api/v1/user/strategies/{id}",
    tag = "策略 (Strategy)",
    security(("bearer_jwt" = [])),
    params(
        ("id" = String, Path, description = "策略实例 ID")
    ),
    responses(
        (status = 200, description = "策略详情获取成功", body = ApiResponse<StrategyResponse>),
        (status = 404, description = "策略不存在"),
        (status = 401, description = "未认证")
    )
)]
pub async fn get_strategy(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<StrategyResponse>>, ApiError> {
    let instance = state.strategy_manager.get_strategy(&user.id, &id).await?;
    Ok(Json(ApiResponse::ok(StrategyResponse::from(&instance))))
}

/// 启动 (部署) 一个新策略
///
/// 接收策略源码与配置，创建策略实例并在引擎中异步启动。
/// 对应 UI 原型中 Strategy Lab 内的 "Deploy" 按钮操作。
#[utoipa::path(
    post,
    path = "/api/v1/user/strategies",
    tag = "策略 (Strategy)",
    security(("bearer_jwt" = [])),
    request_body = StartStrategyRequest,
    responses(
        (status = 200, description = "策略部署成功，返回实例 ID", body = ApiResponse<String>),
        (status = 400, description = "请求参数错误"),
        (status = 401, description = "未认证")
    )
)]
pub async fn deploy_strategy(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<StartStrategyRequest>,
) -> Result<Json<ApiResponse<String>>, ApiError> {
    use okane_core::common::TimeFrame;
    use okane_core::strategy::entity::EngineType;
    use okane_manager::strategy::StartRequest;

    // 解析 TimeFrame
    let timeframe: TimeFrame = req
        .timeframe
        .parse()
        .map_err(|e: String| ApiError::BadRequest(e))?;

    // 解析 EngineType
    let engine_type: EngineType = req
        .engine_type
        .parse()
        .map_err(|e: String| ApiError::BadRequest(e))?;

    // Base64 解码源码
    let source = base64_decode(&req.source_base64)
        .map_err(|e| ApiError::BadRequest(format!("Base64 解码失败: {}", e)))?;

    // IDOR Check: Ensure the strategy binds to an account owner by this user
    if req.account_id != user.id && !req.account_id.starts_with(&format!("{}_", user.id)) {
        tracing::warn!("IDOR attempt: user {} tried to deploy strategy on account {}", user.id, req.account_id);
        return Err(ApiError::Forbidden(format!("Account {} does not belong to user {}", req.account_id, user.id)));
    }

    let start_req = StartRequest {
        symbol: req.symbol,
        account_id: req.account_id,
        timeframe,
        engine_type,
        source,
    };

    let instance_id = state
        .strategy_manager
        .start_strategy(&user.id, start_req)
        .await?;

    Ok(Json(ApiResponse::ok(instance_id)))
}

/// 停止一个正在运行的策略
///
/// 中止策略协程并更新持久化状态为 Stopped。
#[utoipa::path(
    post,
    path = "/api/v1/user/strategies/{id}/stop",
    tag = "策略 (Strategy)",
    security(("bearer_jwt" = [])),
    params(
        ("id" = String, Path, description = "策略实例 ID")
    ),
    responses(
        (status = 200, description = "策略已停止"),
        (status = 404, description = "策略不存在"),
        (status = 401, description = "未认证")
    )
)]
pub async fn stop_strategy(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<String>>, ApiError> {
    state.strategy_manager.stop_strategy(&user.id, &id).await?;
    Ok(Json(ApiResponse::ok("策略已停止".to_string())))
}

// ============================================================
//  辅助函数
// ============================================================

/// 简单的 Base64 解码 (不引入额外依赖)
fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let input = input.trim_end_matches('=');
    let mut output = Vec::with_capacity(input.len() * 3 / 4);

    let mut buf: u32 = 0;
    let mut bits: u32 = 0;

    for byte in input.bytes() {
        let val = TABLE
            .iter()
            .position(|&b| b == byte)
            .ok_or_else(|| format!("非法 Base64 字符: {}", byte as char))?
            as u32;
        buf = (buf << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }

    Ok(output)
}
