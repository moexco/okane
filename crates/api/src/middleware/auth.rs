//! # 鉴权中间件
//!
//! 提供基于 JWT 的身份验证与细粒度角色控制（RBAC）。

use axum::extract::{FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::Response;
use jsonwebtoken::{decode, DecodingKey, Validation};

use crate::types::Claims;
use crate::error::ApiError;
use crate::server::AppState;
use okane_core::store::port::UserRole;

const JWT_SECRET: &str = "YOUR_SUPER_SECRET_KEY"; // TODO: 生产环境应从配置读取

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
            if !s.starts_with("Bearer ") {
                tracing::warn!("Invalid Bearer format: {}", s);
                return Err(ApiError::Unauthorized("Invalid Bearer format".into()));
            }
            s[7..].to_string()
        }
        None => {
            tracing::warn!("Missing Authorization header");
            return Err(ApiError::Unauthorized("Missing Authorization header".into()));
        }
    };

    let claims = match verify_jwt(&token) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("JWT verification failed: {:?}", e);
            return Err(e);
        }
    };
    
    // 检查用户是否存在，以及是否因为初次登录被锁在"强制改密码"状态
    // 如果强制改密码，且当前访问的不是 /api/v1/auth/change_password 接口，则拒绝
    let user = state
        .system_store
        .get_user(&claims.sub)
        .await
        .map_err(|e| ApiError::Internal(format!("DB Error: {}", e)))?
        .ok_or_else(|| ApiError::Unauthorized("User not found".into()))?;

    if user.force_password_change && req.uri().path() != "/api/v1/auth/change_password" {
        return Err(ApiError::Forbidden("You must change your password before using the API".into()));
    }

    // 将用户信息注入 request extensions
    // 以便 downstream handlers 用 `Extension<User>` 提取
    req.extensions_mut().insert(user);
    req.extensions_mut().insert(claims);

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
pub fn verify_jwt(token: &str) -> Result<Claims, ApiError> {
    let mut validation = Validation::default();
    validation.set_required_spec_claims(&["exp", "sub"]);

    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(JWT_SECRET.as_ref()),
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
