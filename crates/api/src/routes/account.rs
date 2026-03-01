//! # 账户资产路由控制器
//!
//! 实现 `/api/v1/user/account/{id}` 路径下的 REST 接口。
//! 对应 UI 原型中 "Key Metrics" 顶部指标卡片区域的数据源。

use axum::extract::{Path, State};
use axum::Json;

use crate::types::{AccountSnapshotResponse, ApiResponse};
use crate::error::ApiError;
use crate::middleware::auth::CurrentUser;
use crate::server::AppState;

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
) -> Result<Json<ApiResponse<AccountSnapshotResponse>>, ApiError> {
    // IDOR Check: Ensures the user owns this account
    if account_id != user.id && !account_id.starts_with(&format!("{}_", user.id)) {
        tracing::warn!("IDOR attempt: user {} tried to access account {}", user.id, account_id);
        return Err(ApiError::Forbidden(format!("Account {} does not belong to user {}", account_id, user.id)));
    }

    let account_id_val = okane_core::trade::entity::AccountId(account_id);
    let snapshot = state.trade_port.get_account(account_id_val).await?;

    // 利用 impl From<AccountSnapshot> for AccountSnapshotResponse 惯用转换
    let response: AccountSnapshotResponse = snapshot.into();

    Ok(Json(ApiResponse::ok(response)))
}
