//! # 账户资产路由控制器
//!
//! 实现 `/api/v1/user/account/{id}` 路径下的 REST 接口。
//! 对应 UI 原型中 "Key Metrics" 顶部指标卡片区域的数据源。

use axum::extract::{Path, State};

use crate::error::ApiError;
use crate::middleware::auth::CurrentUser;
use crate::server::AppState;
use crate::types::{AccountSnapshotResponse, ApiResponse, ApiResult, CreateAccountRequest};
use rust_decimal::Decimal;
use std::str::FromStr;

/// 创建并绑定新的金融账号
///
/// 遵循“无主账号不准开立”原则。创建即绑定，严禁共用。
#[utoipa::path(
    post,
    path = "/api/v1/user/account",
    tag = "账户 (Account)",
    security(("bearer_jwt" = [])),
    request_body = CreateAccountRequest,
    responses(
        (status = 201, description = "开户成功", body = ApiResponse<String>),
        (status = 409, description = "账号已存在"),
        (status = 401, description = "未认证")
    )
)]
pub async fn register_account(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    axum::Json(req): axum::Json<CreateAccountRequest>,
) -> Result<ApiResult<String>, ApiError> {
    // 1. 检查账号是否已存在
    if state
        .system_store
        .get_account_owner(&req.account_id)
        .await
        .map_err(|e| ApiError::database(format!("failed to check account owner: {}", e)))?
        .is_some()
    {
        return Err(ApiError::BadRequest(format!(
            "Account {} already exists and is owned by someone",
            req.account_id
        )));
    }

    // 2. 初始化交易引擎账户。先完成易失败的非持久化步骤，避免把绑定状态写成半成功。
    let initial_balance = if let Some(bal_str) = req.initial_balance {
        Decimal::from_str(&bal_str)
            .map_err(|_| ApiError::BadRequest("Invalid initial balance format".to_string()))?
    } else {
        Decimal::ZERO
    };

    let acc_id = okane_core::trade::entity::AccountId(req.account_id.clone());
    state
        .trade_port
        .ensure_account(acc_id, initial_balance)
        .await
        .map_err(|e| ApiError::runtime(format!("failed to initialize trade account: {}", e)))?;

    // 3. 绑定账号到当前用户
    state
        .system_store
        .bind_account(&user.id, &req.account_id)
        .await
        .map_err(|e| ApiError::database(format!("failed to bind account: {}", e)))?;

    tracing::info!(
        "User {} created and bound account {}",
        user.id,
        req.account_id
    );
    Ok(ApiResult(req.account_id))
}

/// 列出当前用户拥有的所有金融账号
#[utoipa::path(
    get,
    path = "/api/v1/user/accounts",
    tag = "账户 (Account)",
    security(("bearer_jwt" = [])),
    responses(
        (status = 200, description = "成功获取账号列表", body = ApiResponse<Vec<String>>),
        (status = 401, description = "未认证")
    )
)]
pub async fn list_accounts(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<ApiResult<Vec<String>>, ApiError> {
    let accounts = state
        .system_store
        .get_user_accounts(&user.id)
        .await
        .map_err(|e| ApiError::database(format!("failed to list accounts: {}", e)))?;

    Ok(ApiResult(accounts))
}

/// 获取指定系统账户的资金与持仓快照
///
/// 返回该账户当前的可用余额、冻结资金、总权益及全量持仓列表。
/// 对应 UI 原型中的 Total Equity / Available Funds / Positions 区域。
#[utoipa::path(
    get,
    path = "/api/v1/user/account/{account_id}",
    tag = "账户 (Account)",
    security(("bearer_jwt" = [])),
    params(
        ("account_id" = String, Path, description = "系统账户 ID")
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
