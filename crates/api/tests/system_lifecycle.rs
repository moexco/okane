mod common;

use chrono::{Duration, Utc};
use common::spawn_test_server;
use okane_api::types::{ApiErrorResponse, ApiResponse, LoginRequest, LoginResponse};
use okane_core::store::port::UserSession;
use reqwest::StatusCode;
use std::time::{Duration as StdDuration, Instant};

#[tokio::test]
async fn test_deterministic_sid_and_global_cleanup() -> anyhow::Result<()> {
    // 1. Setup
    let (addr, system_store, _temp_dir, _stocks, _feed) = spawn_test_server().await?;
    let client = reqwest::Client::new();

    // 2. Create User A (needed for FK constraint)
    let user_a_id = "user_a";
    let user_a = okane_core::store::port::User {
        id: user_a_id.to_string(),
        name: "User A".to_string(),
        password_hash: "noop".to_string(),
        role: okane_core::store::port::UserRole::Standard,
        force_password_change: false,
        created_at: Utc::now(),
    };
    system_store.save_user(&user_a).await?;

    let expired_sid = "expired_sid_a";
    let expired_session = UserSession {
        id: expired_sid.to_string(),
        user_id: user_a_id.to_string(),
        client_id: "client_a".to_string(),
        current_token_id: "token_a".to_string(),
        expires_at: Utc::now() - Duration::hours(1),
        is_revoked: false,
        created_at: Utc::now() - Duration::hours(2),
    };
    system_store.save_session(&expired_session).await?;

    // Verify User A session is in DB
    let db_session = system_store.get_session(expired_sid).await?;
    assert!(db_session.is_some());

    // 3. User Admin logins (triggers cleanup)
    let login_req = LoginRequest {
        username: "admin".to_string(),
        password: "test_admin_pwd".to_string(), // From common/mod.rs
        client_id: "client_b".to_string(),
    };

    let response = client
        .post(format!("{}/api/v1/auth/login", addr))
        .json(&login_req)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    // 第一次登录的令牌稍后将失效，因为我们要做第二次登录来验证 SID 复用
    let _first_login_res: ApiResponse<LoginResponse> = response.json().await?;

    // 4. Verify Global Cleanup
    // User A's expired session should be deleted from DB because Admin logged in
    let db_session_after = system_store.get_session(expired_sid).await?;
    assert!(
        db_session_after.is_none(),
        "Expired session of User A should have been cleaned up by Admin's login"
    );

    // 5. Verify Deterministic SID
    // Login again with same client_id, it should result in same SID in token
    // This will rotate the JTI, making the first login's tokens invalid!
    let response2 = client
        .post(format!("{}/api/v1/auth/login", addr))
        .json(&login_req)
        .send()
        .await?;
    assert_eq!(response2.status(), StatusCode::OK);
    let login_res: ApiResponse<LoginResponse> = response2.json().await?;

    // Check DB sessions for admin. Should only be 1.
    let active_sessions = system_store.list_active_sessions().await?;
    let admin_sessions: Vec<_> = active_sessions
        .iter()
        .filter(|s| s.user_id == "admin")
        .collect();
    assert_eq!(
        admin_sessions.len(),
        1,
        "There should be exactly one active session for admin due to deterministic SID"
    );

    // 6. Verify Global Cleanup during Refresh
    // Create another expired session
    let expired_sid_2 = "expired_refresh_sid";
    let expired_session_2 = UserSession {
        id: expired_sid_2.to_string(),
        user_id: user_a_id.to_string(),
        client_id: "client_refresh".to_string(),
        current_token_id: "token_refresh".to_string(),
        expires_at: Utc::now() - Duration::hours(1),
        is_revoked: false,
        created_at: Utc::now() - Duration::hours(2),
    };
    system_store.save_session(&expired_session_2).await?;
    let login_data = login_res
        .data
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Login data should be present"))?;

    // Trigger refresh for Admin
    let refresh_req = client
        .post(format!("{}/api/v1/auth/refresh", addr))
        .bearer_auth(&login_data.refresh_token)
        .send()
        .await?;

    assert_eq!(refresh_req.status(), StatusCode::OK);
    let refresh_res: ApiResponse<LoginResponse> = refresh_req.json().await?;
    assert!(
        refresh_res.latency_ms.is_some(),
        "Refresh response should contain latency_ms"
    );

    // Check if expired_sid_2 is gone from DB
    let db_session_after_refresh = system_store.get_session(expired_sid_2).await?;
    // 注意：根据最新需求，refresh 接口不再清理全局过期 Session，所以这里应该是 Some
    assert!(
        db_session_after_refresh.is_some(),
        "Expired session should NOT be cleaned up by refresh anymore"
    );

    // 7. Verify 10s Lockout on Login Failure
    let fail_req = LoginRequest {
        username: "admin".to_string(),
        password: "wrong_password".to_string(),
        client_id: "client_fail".to_string(),
    };

    let fail_start = Instant::now();
    let fail_response = client
        .post(format!("{}/api/v1/auth/login", addr))
        .json(&fail_req)
        .send()
        .await?;
    let fail_elapsed = fail_start.elapsed();

    assert_eq!(fail_response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        fail_elapsed < StdDuration::from_secs(2),
        "Login failure should be fast after removing DoS-prone latency injection, took {:?}",
        fail_elapsed
    );

    let fail_res: ApiErrorResponse = fail_response.json().await?;
    assert!(
        fail_res.latency_ms.is_some(),
        "Error response should also contain latency_ms"
    );

    Ok(())
}
