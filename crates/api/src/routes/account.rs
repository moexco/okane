//! # 账户资产路由控制器
//!
//! 实现 `/api/v1/user/account/{id}` 路径下的 REST 接口。
//! 对应 UI 原型中 "Key Metrics" 顶部指标卡片区域的数据源。

use axum::extract::{Path, State};

use crate::error::ApiError;
use crate::middleware::auth::CurrentUser;
use crate::server::AppState;
use crate::types::{
    AccountProfileResponse, AccountSnapshotResponse, ApiResponse, ApiResult, CreateAccountRequest,
};
use rust_decimal::Decimal;
use std::str::FromStr;
use uuid::Uuid;

/// 创建并绑定新的逻辑交易账号
///
/// 逻辑交易账号是策略、订单、成交、持仓和历史报告的统一载体。
#[utoipa::path(
    post,
    path = "/api/v1/user/account",
    tag = "账户 (Account)",
    security(("bearer_jwt" = [])),
    request_body = CreateAccountRequest,
    responses(
        (status = 201, description = "开户成功", body = ApiResponse<AccountProfileResponse>),
        (status = 409, description = "账号已存在"),
        (status = 401, description = "未认证")
    )
)]
pub async fn register_account(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    axum::Json(req): axum::Json<CreateAccountRequest>,
) -> Result<ApiResult<AccountProfileResponse>, ApiError> {
    let account_type = req.account_type.trim().to_lowercase();
    if account_type.is_empty() {
        return Err(ApiError::BadRequest("type is required".to_string()));
    }

    if req.account_name.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "account_name is required".to_string(),
        ));
    }

    if !req.config.is_object() {
        return Err(ApiError::BadRequest("config must be a JSON object".to_string()));
    }

    let mut config = req.config;
    let initial_balance = if let Some(balance_str) = config
        .get("initial_balance")
        .and_then(serde_json::Value::as_str)
    {
        Decimal::from_str(balance_str)
            .map_err(|_| ApiError::BadRequest("invalid config.initial_balance".to_string()))?
    } else {
        Decimal::ZERO
    };

    if let Some(config_object) = config.as_object_mut() {
        config_object
            .entry("initial_balance".to_string())
            .or_insert_with(|| serde_json::Value::String(initial_balance.to_string()));
    }

    let account_id = format!("acct_{}", Uuid::new_v4().simple());
    let acc_id = okane_core::trade::entity::AccountId(account_id.clone());
    state
        .trade_port
        .ensure_account(acc_id, initial_balance)
        .await
        .map_err(|e| ApiError::runtime(format!("failed to initialize trade account: {}", e)))?;

    state
        .system_store
        .bind_account(
            &user.id,
            &account_id,
            req.account_name.trim(),
            &account_type,
            config.clone(),
        )
        .await
        .map_err(|e| ApiError::database(format!("failed to bind account: {}", e)))?;

    tracing::info!(
        "User {} created account {} of type {}",
        user.id,
        account_id,
        account_type
    );

    Ok(ApiResult(AccountProfileResponse {
        account_id,
        account_name: req.account_name,
        account_type,
        config,
        created_at: chrono::Utc::now().to_rfc3339(),
    }))
}

/// 列出当前用户拥有的所有逻辑交易账号
#[utoipa::path(
    get,
    path = "/api/v1/user/accounts",
    tag = "账户 (Account)",
    security(("bearer_jwt" = [])),
    responses(
        (status = 200, description = "成功获取账号列表", body = ApiResponse<Vec<AccountProfileResponse>>),
        (status = 401, description = "未认证")
    )
)]
pub async fn list_accounts(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<ApiResult<Vec<AccountProfileResponse>>, ApiError> {
    let accounts = state
        .system_store
        .get_user_account_profiles(&user.id)
        .await
        .map_err(|e| ApiError::database(format!("failed to list accounts: {}", e)))?;

    Ok(ApiResult(accounts.into_iter().map(Into::into).collect()))
}

/// 获取指定逻辑交易账号的资金与持仓快照
///
/// 返回该账户当前的可用余额、冻结资金、总权益及全量持仓列表。
/// 对应 UI 原型中的 Total Equity / Available Funds / Positions 区域。
#[utoipa::path(
    get,
    path = "/api/v1/user/account/{account_id}",
    tag = "账户 (Account)",
    security(("bearer_jwt" = [])),
    params(
        ("account_id" = String, Path, description = "逻辑交易账号 ID")
    ),
    responses(
        (status = 200, description = "成功获取账户快照", body = ApiResponse<AccountSnapshotResponse>),
        (status = 404, description = "账户不存在"),
        (status = 401, description = "未认证")
    )
)]
pub async fn get_account_snapshot(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(account_id): Path<String>,
) -> Result<ApiResult<AccountSnapshotResponse>, ApiError> {
    // IDOR Check: Ensures the user owns this account strictly via DB
    let is_owner = state
        .system_store
        .verify_account_ownership(&user.id, &account_id)
        .await
        .map_err(|e| ApiError::database(format!("database error: {}", e)))?;
    if !is_owner {
        tracing::warn!(
            "IDOR attempt: user {} tried to access account {}",
            user.id,
            account_id
        );
        return Err(ApiError::Forbidden(format!(
            "Account {} does not belong to user {}. Ownership required.",
            account_id, user.id
        )));
    }

    let account_id_val = okane_core::trade::entity::AccountId(account_id);
    let snapshot = state.trade_port.get_account(account_id_val).await?;

    // 利用 impl From<AccountSnapshot> for AccountSnapshotResponse 惯用转换
    let response: AccountSnapshotResponse = snapshot.into();

    Ok(ApiResult(response))
}
