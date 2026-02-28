//! # 管理员专有路由控制器
//!
//! 提供系统级别的用户管理、配置控制等能力。
//! 对应的路由受 `auth_middleware` 和 `require_admin` 中间件验证保护。

use axum::extract::State;
use axum::Json;
use chrono::Utc;

use crate::types::{ApiResponse, CreateUserRequest, UserResponse};
use crate::error::ApiError;
use crate::server::AppState;
use okane_core::store::port::{User, UserRole};
use serde::Deserialize;

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateSettingsRequest {
    pub setting_key: String,
    pub setting_value: String,
}

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
    Json(req): Json<CreateUserRequest>,
) -> Result<Json<ApiResponse<UserResponse>>, ApiError> {
    // 1. 判断 ID 是否存在
    tracing::info!("Received create_user request for ID: {}", req.id);
    let existing = state
        .system_store
        .get_user(&req.id)
        .await
        .map_err(|e| ApiError::Internal(format!("DB Checked failed: {}", e)))?;

    if existing.is_some() {
        tracing::warn!("User ID {} already exists!", req.id);
        return Err(ApiError::BadRequest("User ID already exists".into()));
    }

    // 2. 角色解析与密码安全哈希
    let role = req
        .role
        .parse::<UserRole>()
        .map_err(ApiError::BadRequest)?;

    let hashed_pwd = bcrypt::hash(&req.password, bcrypt::DEFAULT_COST)
        .map_err(|_| ApiError::Internal("Failed to hash new user password".into()))?;

    // 3. 构造并保存
    let new_user = User {
        id: req.id,
        name: req.name,
        password_hash: hashed_pwd,
        role,
        force_password_change: true, // 新用户默认被标记为强制改密码
        created_at: Utc::now(),
    };

    state
        .system_store
        .save_user(&new_user)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to save new user to database: {}", e)))?;

    Ok(Json(ApiResponse::ok(UserResponse::from(&new_user))))
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
    State(_state): State<AppState>,
    Json(req): Json<UpdateSettingsRequest>,
) -> Json<ApiResponse<String>> {
    tracing::info!("Admin updating setting '{}' to '{}'", req.setting_key, req.setting_value);
    // TODO: Connect this to actual config hot-reloading or SystemStore key-value settings table
    Json(ApiResponse::ok("ok".to_string()))
}
