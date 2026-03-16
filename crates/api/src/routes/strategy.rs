//! # 策略管理路由控制器
//!
//! 实现 `/api/v1/user/strategies` 路径下的 REST 接口。
//! 对应 UI 原型中的策略列表、部署/停止、以及 Strategy Lab 的代码保存逻辑。

use axum::extract::{Path, Query, State};
use serde::Deserialize;

use crate::types::{ApiResponse, ApiResult, Page, SaveStrategySourceRequest, StartStrategyRequest, StrategyResponse};
use crate::error::ApiError;
use crate::server::AppState;
use crate::middleware::auth::CurrentUser;

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
        (status = 200, description = "策略列表获取成功", body = ApiResponse<Page<StrategyResponse>>),
        (status = 401, description = "未认证")
    )
)]
pub async fn list_strategies(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(query): Query<ListStrategiesQuery>,
) -> Result<ApiResult<Page<StrategyResponse>>, ApiError> {
    let mut instances = state.strategy_manager.list_strategies(&user.id).await?;
    
    // Sort by created_at descending (newest first)
    instances.sort_by_key(|b| std::cmp::Reverse(b.created_at));
    
    let total = instances.len();
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(50).min(500);
    
    let paginated_instances: Vec<_> = instances.into_iter()
        .skip(offset)
        .take(limit)
        .collect();

    let responses: Vec<StrategyResponse> = paginated_instances.iter().map(StrategyResponse::from).collect();

    Ok(ApiResult(Page::new(responses, total, offset, limit)))
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
) -> Result<ApiResult<StrategyResponse>, ApiError> {
    let instance = state.strategy_manager.get_strategy(&user.id, &id).await?;
    Ok(ApiResult(StrategyResponse::from(&instance)))
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
    axum::Json(req): axum::Json<StartStrategyRequest>,
) -> Result<ApiResult<String>, ApiError> {
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
    use base64::prelude::{Engine as _, BASE64_STANDARD};
    let source = BASE64_STANDARD
        .decode(&req.source_base64)
        .map_err(|e| ApiError::BadRequest(format!("base64 decode failed: {}", e)))?;

    // IDOR Check: Ensures the strategy binds to an account owned by this user strictly via DB check.
    let is_owner = state.system_store.verify_account_ownership(&user.id, &req.account_id).await
        .map_err(|e| ApiError::Internal(format!("database error during permission check: {}", e)))?;
    if !is_owner {
        tracing::warn!("IDOR attempt: user {} tried to deploy strategy on account {}", user.id, req.account_id);
        return Err(ApiError::Forbidden(format!("account {} does not belong to user {}. please register the account first.", req.account_id, user.id)));
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

    Ok(ApiResult(instance_id))
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
) -> Result<ApiResult<String>, ApiError> {
    state.strategy_manager.stop_strategy(&user.id, &id).await?;
    Ok(ApiResult("策略已停止".to_string()))
}

/// 更新策略源码
///
/// 更新处于非运行状态的策略源码。
#[utoipa::path(
    put,
    path = "/api/v1/user/strategies/{id}",
    tag = "策略 (Strategy)",
    security(("bearer_jwt" = [])),
    request_body = SaveStrategySourceRequest,
    params(
        ("id" = String, Path, description = "策略实例 ID")
    ),
    responses(
        (status = 200, description = "策略更新成功"),
        (status = 400, description = "请求参数错误或策略正在运行"),
        (status = 404, description = "策略不存在"),
        (status = 401, description = "未认证")
    )
)]
pub async fn update_strategy(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    axum::Json(req): axum::Json<SaveStrategySourceRequest>,
) -> Result<ApiResult<String>, ApiError> {
    use base64::prelude::{Engine as _, BASE64_STANDARD};
    let source = BASE64_STANDARD
        .decode(&req.source_base64)
        .map_err(|e| ApiError::BadRequest(format!("base64 decode failed: {}", e)))?;

    state.strategy_manager.update_strategy(&user.id, &id, source).await?;
    Ok(ApiResult("策略已更新".to_string()))
}

/// 删除策略
///
/// 删除处于非运行状态的策略记录。
#[utoipa::path(
    delete,
    path = "/api/v1/user/strategies/{id}",
    tag = "策略 (Strategy)",
    security(("bearer_jwt" = [])),
    params(
        ("id" = String, Path, description = "策略实例 ID")
    ),
    responses(
        (status = 200, description = "策略已删除"),
        (status = 400, description = "策略正在运行"),
        (status = 404, description = "策略不存在"),
        (status = 401, description = "未认证")
    )
)]
pub async fn delete_strategy(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Result<ApiResult<String>, ApiError> {
    state.strategy_manager.delete_strategy(&user.id, &id).await?;
    Ok(ApiResult("策略已删除".to_string()))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct GetLogsQuery {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// 查询策略运行时日志
#[utoipa::path(
    get,
    path = "/api/v1/user/strategies/{id}/logs",
    tag = "策略 (Strategy)",
    security(("bearer_jwt" = [])),
    params(
        ("id" = String, Path, description = "策略实例 ID"),
        ("limit" = Option<usize>, Query, description = "返回条数，默认 100"),
        ("offset" = Option<usize>, Query, description = "跳过条数，默认 0")
    ),
    responses(
        (status = 200, description = "日志获取成功", body = ApiResponse<Page<okane_core::strategy::entity::StrategyLogEntry>>),
        (status = 404, description = "策略不存在")
    )
)]
pub async fn get_strategy_logs(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Query(query): Query<GetLogsQuery>,
) -> Result<ApiResult<Page<okane_core::strategy::entity::StrategyLogEntry>>, ApiError> {
    // 权限与存在性检查
    state.strategy_manager.get_strategy(&user.id, &id).await?;

    let limit = query.limit.unwrap_or(100).min(500);
    let offset = query.offset.unwrap_or(0);

    let logs = state.strategy_manager.get_logs(&user.id, &id, limit, offset).await
        .map_err(|e| ApiError::Internal(format!("failed to query logs: {}", e)))?;

    // 由于目前底层的 StrategyLogPort 不直接返回总数，且日志量极大，
    // 这里暂时返回当前获得的数量作为总数（或者可以用特殊的标识如 -1 探测是否有下一页），
    // 考虑到用户需求是“哪怕是分页也放在 data 字段里”，这里采用 Page 包装。
    // 在真实生产环境中，应该给 query_logs 增加 count 支持。
    let total = if logs.len() < limit { offset + logs.len() } else { 1000000 }; // 这里的 1000000 仅作示意

    Ok(ApiResult(Page::new(logs, total, offset, limit)))
}
