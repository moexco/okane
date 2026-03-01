//! # 身份验证路由控制器
//!
//! 实现登录、密码修改等鉴权相关接口。

use axum::extract::State;
use axum::Json;
use chrono::Utc;
use jsonwebtoken::{encode, EncodingKey, Header};

use crate::types::{ApiResponse, ChangePasswordRequest, Claims, LoginRequest, LoginResponse};
use crate::error::ApiError;
use crate::middleware::auth::CurrentUser;
use crate::server::AppState;

const JWT_EXPIRES_IN: u64 = 86400 * 7; // 7 days

/// 用户登录
///
/// 验证用户名和密码，颁发 JWT Token。
#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    tag = "鉴权 (Auth)",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "登录成功", body = ApiResponse<LoginResponse>),
        (status = 401, description = "用户名或密码错误")
    )
)]
pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<ApiResponse<LoginResponse>>, ApiError> {
    // 1. 获取用户
    let user = state
        .system_store
        .get_user(&req.username)
        .await
        .map_err(|e| ApiError::Internal(format!("DB error: {}", e)))?;

    let user = match user {
        Some(u) => u,
        None => return Err(ApiError::Unauthorized("Invalid username or password".into())),
    };

    // 2. 验证密码
    let valid = bcrypt::verify(&req.password, &user.password_hash)
        .unwrap_or(false);

    if !valid {
        return Err(ApiError::Unauthorized("Invalid username or password".into()));
    }

    // 3. 生成 JWT
    let exp = Utc::now().timestamp() as usize + JWT_EXPIRES_IN as usize;
    let claims = Claims {
        sub: user.id.clone(),
        role: user.role.to_string(),
        exp,
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(state.app_config.server.jwt_secret.as_ref()),
    )
    .map_err(|_| ApiError::Internal("Failed to generate token".into()))?;

    Ok(Json(ApiResponse::ok(LoginResponse {
        token,
        expires_in: JWT_EXPIRES_IN,
    })))
}

/// 修改密码
///
/// 验证旧密码并设立新密码。如果用户标记为强制修改密码，此操作会解除该状态。
#[utoipa::path(
    post,
    path = "/api/v1/auth/change_password",
    tag = "鉴权 (Auth)",
    security(("bearer_jwt" = [])),
    request_body = ChangePasswordRequest,
    responses(
        (status = 200, description = "密码修改成功", body = ApiResponse<String>),
        (status = 401, description = "原密码错误或未认证")
    )
)]
pub async fn change_password(
    State(state): State<AppState>,
    CurrentUser(mut user): CurrentUser,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<Json<ApiResponse<String>>, ApiError> {
    // 1. 验证旧密码
    tracing::info!("Attempting to change password for user: {}, force_change: {}", user.id, user.force_password_change);
    let valid = bcrypt::verify(&req.old_password, &user.password_hash)
        .unwrap_or(false);

    if !valid {
        tracing::warn!("Failed old password validation for user {}", user.id);
        return Err(ApiError::Unauthorized("Invalid old password".into()));
    }

    // 2. 生成新密码的 Hash
    let new_hashed = bcrypt::hash(&req.new_password, bcrypt::DEFAULT_COST)
        .map_err(|_| ApiError::Internal("Failed to hash new password".into()))?;

    // 3. 更新 User 实体
    user.password_hash = new_hashed;
    user.force_password_change = false; // 取消强制修改密码要求

    state
        .system_store
        .save_user(&user)
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to save new password: {}", e)))?;

    Ok(Json(ApiResponse::ok("Password changed successfully".into())))
}
