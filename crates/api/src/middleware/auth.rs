//! # 鉴权中间件
//!
//! 提供基于 JWT 的身份验证与细粒度角色控制（RBAC）。

use axum::extract::{FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::Response;
use chrono::Utc;
use jsonwebtoken::{decode, DecodingKey, Validation};

use crate::types::Claims;
use crate::error::ApiError;
use crate::server::AppState;
use okane_core::store::port::UserRole;

/// 提取并验证 Authorization: Bearer <token>
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let auth_header = req.headers().get(axum::http::header::AUTHORIZATION);
    
    let token = match auth_header {
        Some(header_val) => {
            let s = header_val.to_str().map_err(|_| ApiError::Unauthorized("Invalid auth header".into()))?;
            match s.strip_prefix("Bearer ") {
                Some(t) => t.to_string(),
                None => return Err(ApiError::Unauthorized("Invalid Bearer format".into())),
            }
        }
        None => {
            // 如果 Header 缺失，尝试从 Query 参数解析 (适配浏览器原生 WebSocket)
            let query_string = req.uri().query().unwrap_or_default();
            let mut token = None;
            for pair in query_string.split('&') {
                let mut parts = pair.split('=');
                if let (Some(key), Some(val)) = (parts.next(), parts.next()) {
                    if key == "token" || key == "access_token" {
                        token = Some(val.to_string());
                        break;
                    }
                }
            }
            token.ok_or_else(|| ApiError::Unauthorized("Missing Authorization header or token in query".into()))?
        }
    };

    // 1. 验证 JWT 基础合法性
    let claims = verify_jwt(&token, &state.app_config.server.jwt_secret)?;
    
    // 2. 检查 Session 状态 (即时撤销核心逻辑)
    // 强制运行时零 DB 读取，内存未命中视为无效
    let session = state.session_cache.get(&claims.sid).map(|s| s.clone());

    let session = session.ok_or_else(|| ApiError::Unauthorized("Session not found".into()))?;

    if session.is_revoked || session.expires_at < Utc::now() {
        return Err(ApiError::Unauthorized("Session has been revoked or expired".into()));
    }

    // 3. 构造 User 实体注入 Context (利用 Claims 中的冗余信息)
    let user = okane_core::store::port::User {
        id: claims.sub.clone(),
        name: "".to_string(), 
        password_hash: "".to_string(),
        role: claims.role.parse().unwrap_or(UserRole::Standard),
        force_password_change: claims.force_password_change,
        created_at: Utc::now(), 
    };

    req.extensions_mut().insert(user);
    req.extensions_mut().insert(claims);

    Ok(next.run(req).await)
}

/// 检查用户是否需要强制修改密码
/// 必须在 `auth_middleware` 之后应用！
pub async fn require_password_changed(
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let user = req
        .extensions()
        .get::<okane_core::store::port::User>()
        .ok_or_else(|| ApiError::Unauthorized("User context not found".into()))?;

    if user.force_password_change {
        return Err(ApiError::Forbidden("You must change your password before using the API".into()));
    }

    Ok(next.run(req).await)
}

/// Admin 级别权限校验中间件
/// 必须在 `auth_middleware` 之后应用！
pub async fn require_admin(
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let user = req
        .extensions()
        .get::<okane_core::store::port::User>()
        .ok_or_else(|| ApiError::Unauthorized("User context not found".into()))?;

    if user.role != UserRole::Admin {
        return Err(ApiError::Forbidden("Admin privileges required".into()));
    }

    Ok(next.run(req).await)
}

/// 验证 JWT 返回强类型 Claims
pub fn verify_jwt(token: &str, secret: &str) -> Result<Claims, ApiError> {
    let mut validation = Validation::default();
    validation.set_required_spec_claims(&["exp", "sub"]);

    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_ref()),
        &validation,
    )
    .map_err(|_| ApiError::Unauthorized("Invalid or expired token".into()))?;

    Ok(token_data.claims)
}

// 在提取器中获取当前用户的快捷方式
pub struct CurrentUser(pub okane_core::store::port::User);

impl<S> FromRequestParts<S> for CurrentUser
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let user = parts
            .extensions
            .get::<okane_core::store::port::User>()
            .cloned()
            .ok_or_else(|| ApiError::Unauthorized("Missing User Context".into()))?;
        Ok(CurrentUser(user))
    }
}
