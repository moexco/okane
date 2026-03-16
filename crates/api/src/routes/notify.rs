use axum::extract::State;
use crate::{
    error::ApiError,
    middleware::auth::CurrentUser,
    server::AppState,
    types::{ApiResponse, ApiResult, NotifyConfigResponse, UpdateNotifyConfigRequest},
};

/// 查询用户通知配置
///
/// 获取当前用户的通知推送偏好。
#[utoipa::path(
    get,
    path = "/api/v1/user/notify-config",
    tag = "通知 (Notify)",
    security(("bearer_jwt" = [])),
    responses(
        (status = 200, description = "成功获取配置", body = ApiResponse<NotifyConfigResponse>),
        (status = 401, description = "未认证")
    )
)]
pub async fn get_notify_config(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<ApiResult<NotifyConfigResponse>, ApiError> {
    let config = state
        .system_store
        .get_user_notify_config(&user.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound("notify config not found".to_string()))?;

    Ok(ApiResult(NotifyConfigResponse::from(config)))
}

/// 更新用户通知配置
///
/// 更新当前用户的通知推送偏好（全量覆盖）。
#[utoipa::path(
    put,
    path = "/api/v1/user/notify-config",
    tag = "通知 (Notify)",
    security(("bearer_jwt" = [])),
    request_body = UpdateNotifyConfigRequest,
    responses(
        (status = 200, description = "配置更新成功", body = ApiResponse<String>),
        (status = 400, description = "参数错误"),
        (status = 401, description = "未认证")
    )
)]
pub async fn update_notify_config(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    axum::Json(req): axum::Json<UpdateNotifyConfigRequest>,
) -> Result<ApiResult<String>, ApiError> {
    let config: okane_core::config::UserNotifyConfig = req.into();

    let valid_channels = ["none", "telegram", "email"];
    if !valid_channels.contains(&config.channel.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "无效的 channel: {}，允许值: {:?}",
            config.channel, valid_channels
        )));
    }

    state
        .system_store
        .save_user_notify_config(&user.id, &config)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(ApiResult("通知配置已更新".to_string()))
}
