#![allow(clippy::unwrap_used)]
mod common;

use reqwest::StatusCode;
use okane_api::types::{ApiResponse, LoginRequest, LoginResponse};
use common::spawn_test_server;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_refresh_token_rotation_and_reuse_detection() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();

    // 1. 登录
    let res = assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "admin".to_string(),
        password: "test_admin_pwd".to_string(),
    }, StatusCode::OK);
    
    let login_data: ApiResponse<LoginResponse> = res.json().await?;
    let tokens = login_data.data.unwrap();
    let rt1 = tokens.refresh_token;

    // 2. 第一次刷新 (合法)
    let res = assert_post!(&client, format!("{}/api/v1/auth/refresh", base_url), Some(&rt1), &serde_json::json!({}), StatusCode::OK);
    let refresh_data1: ApiResponse<LoginResponse> = res.json().await?;
    let tokens2 = refresh_data1.data.unwrap();
    let rt2 = tokens2.refresh_token;

    assert!(rt1 != rt2, "Refresh token must rotate");

    // 3. 第二次刷新 (合法，使用 rt2)
    let res = assert_post!(&client, format!("{}/api/v1/auth/refresh", base_url), Some(&rt2), &serde_json::json!({}), StatusCode::OK);
    let refresh_data2: ApiResponse<LoginResponse> = res.json().await?;
    let tokens3 = refresh_data2.data.unwrap();
    let rt3 = tokens3.refresh_token;

    assert!(rt2 != rt3, "Refresh token must rotate again");

    // 4. 重放攻击探测 (使用已作废的 rt1 尝试刷新)
    // 根据逻辑，这应该触发重放攻击探测，并撤销该用户所有 Session
    let res = client.post(format!("{}/api/v1/auth/refresh", base_url))
        .bearer_auth(&rt1)
        .json(&serde_json::json!({}))
        .send()
        .await?;
    
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "Reused RT must be rejected");
    let error_body: ApiResponse<String> = res.json().await?;
    assert!(error_body.error.unwrap().contains("reuse detected"), "Should mention reuse detection");

    // 5. 验证熔断 (使用最新合法的 rt3 尝试刷新，由于 rt1 的重用，rt3 应该也已失效)
    let res = client.post(format!("{}/api/v1/auth/refresh", base_url))
        .bearer_auth(&rt3)
        .json(&serde_json::json!({}))
        .send()
        .await?;
    
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "Active RT must be revoked after reuse detection");

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_password_change_revokes_all_sessions() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();

    // 1. 登录
    let res = assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "admin".to_string(),
        password: "test_admin_pwd".to_string(),
    }, StatusCode::OK);
    let tokens = res.json::<ApiResponse<LoginResponse>>().await?.data.unwrap();
    let at1 = tokens.access_token;

    // 2. 修改密码
    let change_req = serde_json::json!({
        "old_password": "test_admin_pwd",
        "new_password": "new_secret_pwd"
    });
    assert_post!(&client, format!("{}/api/v1/auth/change_password", base_url), Some(&at1), &change_req, StatusCode::OK);

    // 3. 验证旧的 AT 立即失效
    let res = client.get(format!("{}/api/v1/user/accounts", base_url))
        .bearer_auth(&at1)
        .send()
        .await?;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "Old AT must be revoked immediately after password change");

    // 4. 用新密码登录并验证
    assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "admin".to_string(),
        password: "new_secret_pwd".to_string(),
    }, StatusCode::OK);

    Ok(())
}
