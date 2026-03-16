//! # 身份验证路由控制器
//!
//! 实现登录、密码修改等鉴权相关接口。

use axum::extract::State;
use chrono::Utc;
use jsonwebtoken::{encode, EncodingKey, Header};

use sha2::{Sha256, Digest};
use crate::types::{ApiResponse, ApiResult, ChangePasswordRequest, Claims, LoginRequest, LoginResponse};
use crate::error::ApiError;
use crate::server::AppState;
use crate::middleware::auth::CurrentUser;

const ACCESS_TOKEN_EXPIRES_IN: i64 = 900; // 15 min
const REFRESH_TOKEN_EXPIRES_IN: i64 = 180 * 24 * 3600; // 180 days

/// 用户登录
///
/// 验证用户名和密码，颁发 Access Token 和 Refresh Token。
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
    axum::Json(req): axum::Json<LoginRequest>,
) -> Result<ApiResult<LoginResponse>, ApiError> {
    // 1. 获取用户
    let user_opt = state
        .system_store
        .get_user(&req.username)
        .await
        .map_err(|e| ApiError::Internal(format!("db error: {}", e)))?;
        
    let user = match user_opt {
        Some(u) => u,
        None => return Err(ApiError::Unauthorized("invalid username or password".into())),
    };

    // 2. 验证密码
    let valid = bcrypt::verify(&req.password, &user.password_hash)
        .map_err(|e| ApiError::Internal(format!("hash verification failed: {}", e)))?;

    if !valid {
        return Err(ApiError::Unauthorized("invalid username or password".into()));
    }

    // 3. 创建或复用 Session (确定性 SID)
    let session_id = generate_deterministic_sid(&user.id, &req.client_id);
    let expires_at = Utc::now() + chrono::Duration::seconds(REFRESH_TOKEN_EXPIRES_IN);
    
    let mut session = if let Some(s) = state.session_cache.get(&session_id) {
        let mut s = s.clone();
        s.expires_at = expires_at;
        s
    } else {
        okane_core::store::port::UserSession {
            id: session_id.clone(),
            user_id: user.id.clone(),
            client_id: req.client_id.clone(),
            current_token_id: "".to_string(), 
            expires_at,
            is_revoked: false,
            created_at: Utc::now(),
        }
    };

    let current_jti = uuid::Uuid::new_v4().to_string();
    session.current_token_id = current_jti.clone();

    // 4. 同步至 DB & RAM (仅在登录成功后进行全局清理)
    state.system_store.save_session(&session).await
        .map_err(|e| ApiError::Internal(format!("failed to save session: {}", e)))?;
    
    // 全局自动净化：仅在登录成功后，顺便清理系统中所有过期数据
    state.system_store.delete_expired_sessions().await.ok();
    
    state.session_cache.insert(session_id.clone(), session.clone());

    // 5. 生成令牌对
    let access_token = generate_access_token(&user, &session_id, &uuid::Uuid::new_v4().to_string(), &state.app_config.server.jwt_secret)?;
    let refresh_token = generate_refresh_token(&user, &session_id, &current_jti, &state.app_config.server.jwt_secret)?;

    Ok(ApiResult(LoginResponse {
        access_token,
        refresh_token,
        expires_in: ACCESS_TOKEN_EXPIRES_IN as u64,
    }))
}

/// 刷新令牌 (Rolling Refresh)
///
/// 使用旧的 Refresh Token 换取新的令牌对。只要在半年内有活跃操作，即可保持登录。
#[utoipa::path(
    post,
    path = "/api/v1/auth/refresh",
    tag = "鉴权 (Auth)",
    security(("bearer_jwt" = [])),
    responses(
        (status = 200, description = "刷新成功", body = ApiResponse<LoginResponse>),
        (status = 401, description = "无效或过期的刷新令牌")
    )
)]
pub async fn refresh(
    State(state): State<AppState>,
    req: axum::extract::Request,
) -> Result<ApiResult<LoginResponse>, ApiError> {
    // 提取并验证 Refresh Token
    let auth_header = req.headers().get(axum::http::header::AUTHORIZATION)
        .ok_or_else(|| ApiError::Unauthorized("missing refresh token".into()))?;
    
    let token_str = auth_header.to_str()
        .map_err(|_| ApiError::Unauthorized("invalid header".into()))?
        .strip_prefix("Bearer ")
        .ok_or_else(|| ApiError::Unauthorized("invalid format".into()))?;

    let claims = crate::middleware::auth::verify_jwt(token_str, &state.app_config.server.jwt_secret)?;

    // 1. 获取 Session (严格零 DB 读取，仅查内存)
    let mut session = state.session_cache.get(&claims.sid)
        .map(|s| s.clone())
        .ok_or_else(|| ApiError::Unauthorized("invalid or expired session. please log in again.".into()))?;

    // 2. 重放攻击探测 (Reuse Detection)
    // 提交的 jti 必须与当前 Session 记录的唯一合法 jti 一致
    if claims.jti != session.current_token_id {
        tracing::error!("reuse detection triggered! user: {}, session: {}, token: {}", claims.sub, claims.sid, claims.jti);
        
        // 熔断：撤销该用户所有 Session (含存储层)
        state.system_store.revoke_all_user_sessions(&claims.sub).await.ok();
        state.session_cache.retain(|_, v| v.user_id != claims.sub);
        
        return Err(ApiError::Unauthorized("token reuse detected. all sessions revoked for security.".into()));
    }

    if session.is_revoked || session.expires_at < Utc::now() {
        return Err(ApiError::Unauthorized("session revoked or expired".into()));
    }

    // 3. 令牌轮转 (Rotation / JTI Rotation)
    let new_jti = uuid::Uuid::new_v4().to_string();
    session.current_token_id = new_jti.clone();
    
    // 同步更新 DB & RAM
    state.system_store.save_session(&session).await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    
    state.session_cache.insert(session.id.clone(), session.clone());

    // 获取用户实体以生成 AT
    let user = state.system_store.get_user(&claims.sub).await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::Unauthorized("user not found".into()))?;

    let access_token = generate_access_token(&user, &session.id, &uuid::Uuid::new_v4().to_string(), &state.app_config.server.jwt_secret)?;
    let refresh_token = generate_refresh_token(&user, &session.id, &new_jti, &state.app_config.server.jwt_secret)?;

    Ok(ApiResult(LoginResponse {
        access_token,
        refresh_token,
        expires_in: ACCESS_TOKEN_EXPIRES_IN as u64,
    }))
}

/// 用户登出
///
/// 撤销当前会话，使关联的 Access Token 和 Refresh Token 失效。
#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    tag = "鉴权 (Auth)",
    security(("bearer_jwt" = [])),
    responses(
        (status = 200, description = "登出成功", body = ApiResponse<String>),
        (status = 401, description = "未登录或会话已过期")
    )
)]
pub async fn logout(
    State(state): State<AppState>,
    crate::middleware::auth::CurrentUser(user_ctx): crate::middleware::auth::CurrentUser,
    axum::Extension(claims): axum::Extension<Claims>,
) -> Result<ApiResult<String>, ApiError> {
    // 1. 在持久化存储中撤销 Session
    state.system_store.revoke_session(&claims.sid).await
        .map_err(|e| ApiError::Internal(format!("failed to revoke session: {}", e)))?;

    // 2. 从内存缓存中即时移除
    state.session_cache.remove(&claims.sid);

    tracing::info!("user logout success: {}, sid: {}", user_ctx.id, claims.sid);

    Ok(ApiResult("Logged out successfully".into()))
}

/// 修改密码
///
/// 验证旧密码并设立新密码。此操作会撤销该用户所有活跃 Session 以确保安全。
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
    CurrentUser(user_ctx): CurrentUser,
    axum::Json(req): axum::Json<ChangePasswordRequest>,
) -> Result<ApiResult<String>, ApiError> {
    // 1. 显式从数据库获取完整用户信息（包含密码哈希）
    let mut user = state.system_store.get_user(&user_ctx.id).await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::Unauthorized("user not found".into()))?;

    // 2. 验证旧密码
    let valid = bcrypt::verify(&req.old_password, &user.password_hash)
        .map_err(|e| ApiError::Internal(format!("hash verification failed: {}", e)))?;

    if !valid {
        return Err(ApiError::Unauthorized("invalid old password".into()));
    }

    // 3. 更新密码
    let new_hashed = bcrypt::hash(&req.new_password, bcrypt::DEFAULT_COST)
        .map_err(|_| ApiError::Internal("failed to hash new password".into()))?;

    user.password_hash = new_hashed;
    user.force_password_change = false;

    state
        .system_store
        .save_user(&user)
        .await
        .map_err(|e| ApiError::Internal(format!("failed to save user: {}", e)))?;

    // 4. 全局注销：撤销该用户所有活跃 Session (强制重新登录)
    state.system_store.revoke_all_user_sessions(&user.id).await
        .map_err(|e| ApiError::Internal(format!("failed to revoke sessions: {}", e)))?;
    
    state.session_cache.retain(|_, v| v.user_id != user.id);

    Ok(ApiResult("Password changed successfully. All other sessions revoked.".into()))
}

fn generate_access_token(user: &okane_core::store::port::User, sid: &str, jti: &str, secret: &str) -> Result<String, ApiError> {
    let expiration = Utc::now()
        .checked_add_signed(chrono::Duration::seconds(ACCESS_TOKEN_EXPIRES_IN))
        .ok_or_else(|| ApiError::Internal("timestamp calculation overflow".into()))?
        .timestamp();

    let claims = Claims {
        sub: user.id.clone(),
        sid: sid.to_string(),
        jti: jti.to_string(),
        exp: expiration.try_into().map_err(|_| ApiError::Internal("timestamp out of bounds".into()))?,
        role: user.role.to_string(),
        force_password_change: user.force_password_change,
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_ref()),
    )
    .map_err(|e| ApiError::Internal(format!("jwt sign failed: {}", e)))
}

fn generate_refresh_token(user: &okane_core::store::port::User, sid: &str, jti: &str, secret: &str) -> Result<String, ApiError> {
    let expiration = Utc::now()
        .checked_add_signed(chrono::Duration::seconds(REFRESH_TOKEN_EXPIRES_IN))
        .ok_or_else(|| ApiError::Internal("timestamp calculation overflow".into()))?
        .timestamp();

    let claims = Claims {
        sub: user.id.clone(),
        sid: sid.to_string(),
        jti: jti.to_string(),
        exp: expiration.try_into().map_err(|_| ApiError::Internal("timestamp out of bounds".into()))?,
        role: user.role.to_string(),
        force_password_change: user.force_password_change,
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_ref()),
    )
    .map_err(|e| ApiError::Internal(format!("jwt refresh sign failed: {}", e)))
}

/// 生成确定性的 Session ID
/// 基于 (user_id + client_id) 的 SHA256 哈希，确保存储幂等性，无需读取 DB 即可实现覆写。
fn generate_deterministic_sid(user_id: &str, client_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(user_id.as_bytes());
    hasher.update(client_id.as_bytes());
    hex::encode(hasher.finalize())
}
