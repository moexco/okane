use crate::error::ApiError;
use crate::server::AppState;
use crate::types::{
    ApiResponse, ApiResult, CreateUserRequest, UpdateSettingsRequest, UserResponse,
};
use axum::extract::State;
use chrono::Utc;
use okane_core::store::port::{User, UserRole};

/// 创建新子账户
///
/// 只有 Admin 角色的用户可以调用此接口为他人创建账户。
#[utoipa::path(
    post,
    path = "/api/v1/admin/users",
    tag = "系统管理 (Admin)",
    security(("bearer_jwt" = [])),
    request_body = CreateUserRequest,
    responses(
        (status = 200, description = "子用户创建成功", body = ApiResponse<UserResponse>),
        (status = 400, description = "无效的请求参数"),
        (status = 401, description = "未认证"),
        (status = 403, description = "无权限执行此操作")
    )
)]
pub async fn create_user(
    State(state): State<AppState>,
    axum::Json(req_json): axum::Json<serde_json::Value>,
) -> Result<ApiResult<UserResponse>, ApiError> {
    let req: CreateUserRequest = serde_json::from_value(req_json.clone())
        .map_err(|e| ApiError::BadRequest(format!("invalid request: {}", e)))?;
    // 1. 判断 ID 是否存在
    tracing::info!("Received create_user request for ID: {}", req.id);
    let existing = state
        .system_store
        .get_user(&req.id)
        .await
        .map_err(|e| ApiError::database(format!("db check failed: {}", e)))?;

    if existing.is_some() {
        tracing::warn!("User ID {} already exists!", req.id);
        return Err(ApiError::BadRequest("User ID already exists".into()));
    }

    // 2. 角色解析与密码安全哈希
    let role = req.role.parse::<UserRole>().map_err(ApiError::BadRequest)?;

    let hashed_pwd = bcrypt::hash(&req.password, bcrypt::DEFAULT_COST)
        .map_err(|_| ApiError::crypto("failed to hash new user password"))?;

    // 3. 构造并保存
    let force_change = req_json
        .get("force_password_change")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let new_user = User {
        id: req.id,
        name: req.name,
        password_hash: hashed_pwd,
        role,
        force_password_change: force_change,
        created_at: Utc::now(),
    };

    state
        .system_store
        .save_user(&new_user)
        .await
        .map_err(|e| ApiError::database(format!("failed to save new user: {}", e)))?;

    Ok(ApiResult(UserResponse::from(&new_user)))
}

/// 更新系统全局设置
///
/// 只有 Admin 角色可以修改应用级配置
#[utoipa::path(
    put,
    path = "/api/v1/admin/settings",
    tag = "系统管理 (Admin)",
    security(("bearer_jwt" = [])),
    request_body = UpdateSettingsRequest,
    responses(
        (status = 200, description = "配置更新成功", body = ApiResponse<String>),
        (status = 500, description = "服务器内部错误")
    )
)]
pub async fn update_settings(
    State(state): State<AppState>,
    axum::Json(req): axum::Json<UpdateSettingsRequest>,
) -> Result<ApiResult<String>, ApiError> {
    tracing::info!(
        "Admin updating setting '{}' to '{}'",
        req.setting_key,
        req.setting_value
    );

    state
        .system_store
        .set_setting(&req.setting_key, &req.setting_value)
        .await
        .map_err(|e| ApiError::database(format!("failed to save setting: {}", e)))?;

    // TODO: Broadcast event to Engine for hot-reloading if applicable
    Ok(ApiResult("ok".to_string()))
}
