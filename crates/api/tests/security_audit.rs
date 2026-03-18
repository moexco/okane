mod common;

use common::spawn_test_server;
use okane_api::types::{ApiResponse, LoginRequest, LoginResponse};
use reqwest::StatusCode;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_refresh_token_rotation_and_reuse_detection() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive, _feed) = spawn_test_server().await?;
    let client = reqwest::Client::new();

    // 1. 登录
    let res = assert_post!(
        &client,
        format!("{}/api/v1/auth/login", base_url),
        None::<&str>,
        &LoginRequest {
            username: "admin".to_string(),
            password: "test_admin_pwd".to_string(),
            client_id: "sec_test_client".to_string(),
        },
        StatusCode::OK
    );

    let login_data: ApiResponse<LoginResponse> = res.json().await?;
    let tokens = login_data
        .data
        .ok_or_else(|| anyhow::anyhow!("login response missing tokens"))?;
    let rt1 = tokens.refresh_token;

    // 2. 第一次刷新 (合法)
    let res = assert_post!(
        &client,
        format!("{}/api/v1/auth/refresh", base_url),
        Some(&rt1),
        &serde_json::json!({}),
        StatusCode::OK
    );
    let refresh_data1: ApiResponse<LoginResponse> = res.json().await?;
    let tokens2 = refresh_data1
        .data
        .ok_or_else(|| anyhow::anyhow!("refresh 1 missing tokens"))?;
    let rt2 = tokens2.refresh_token;

    assert!(rt1 != rt2, "Refresh token must rotate");

    // 3. 第二次刷新 (合法，使用 rt2)
    let res = assert_post!(
        &client,
        format!("{}/api/v1/auth/refresh", base_url),
        Some(&rt2),
        &serde_json::json!({}),
        StatusCode::OK
    );
    let refresh_data2: ApiResponse<LoginResponse> = res.json().await?;
    let tokens3 = refresh_data2
        .data
        .ok_or_else(|| anyhow::anyhow!("refresh 2 missing tokens"))?;
    let rt3 = tokens3.refresh_token;

    assert!(rt2 != rt3, "Refresh token must rotate again");

    // 4. 重放攻击探测 (使用已作废的 rt1 尝试刷新)
    // 根据逻辑，这应该触发重放攻击探测，并撤销该用户所有 Session
    let res = client
        .post(format!("{}/api/v1/auth/refresh", base_url))
        .bearer_auth(&rt1)
        .json(&serde_json::json!({}))
        .send()
        .await?;

    assert_eq!(
        res.status(),
        StatusCode::UNAUTHORIZED,
        "Reused RT must be rejected"
    );
    let error_body: ApiResponse<String> = res.json().await?;
    assert!(
        error_body
            .error
            .as_ref()
            .map(|e| e.contains("reuse detected"))
            .unwrap_or(false),
        "error should mention reuse detection"
    );

    // 5. 验证熔断 (使用最新合法的 rt3 尝试刷新，由于 rt1 的重用，rt3 应该也已失效)
    let res = client
        .post(format!("{}/api/v1/auth/refresh", base_url))
        .bearer_auth(&rt3)
        .json(&serde_json::json!({}))
        .send()
        .await?;

    assert_eq!(
        res.status(),
        StatusCode::UNAUTHORIZED,
        "Active RT must be revoked after reuse detection"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_password_change_revokes_all_sessions() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive, _feed) = spawn_test_server().await?;
    let client = reqwest::Client::new();

    // 1. 登录
    let res = assert_post!(
        &client,
        format!("{}/api/v1/auth/login", base_url),
        None::<&str>,
        &LoginRequest {
            username: "admin".to_string(),
            password: "test_admin_pwd".to_string(),
            client_id: "sec_test_client".to_string(),
        },
        StatusCode::OK
    );
    let tokens = res
        .json::<ApiResponse<LoginResponse>>()
        .await?
        .data
        .ok_or_else(|| anyhow::anyhow!("login missing tokens"))?;
    let at1 = tokens.access_token;

    // 2. 修改密码
    let change_req = serde_json::json!({
        "old_password": "test_admin_pwd",
        "new_password": "new_secret_pwd"
    });
    assert_post!(
        &client,
        format!("{}/api/v1/auth/change_password", base_url),
        Some(&at1),
        &change_req,
        StatusCode::OK
    );

    // 3. 验证旧的 AT 立即失效
    let res = client
        .get(format!("{}/api/v1/user/accounts", base_url))
        .bearer_auth(&at1)
        .send()
        .await?;
    assert_eq!(
        res.status(),
        StatusCode::UNAUTHORIZED,
        "Old AT must be revoked immediately after password change"
    );

    // 4. 用新密码登录并验证
    assert_post!(
        &client,
        format!("{}/api/v1/auth/login", base_url),
        None::<&str>,
        &LoginRequest {
            username: "admin".to_string(),
            password: "new_secret_pwd".to_string(),
            client_id: "sec_test_client".to_string(),
        },
        StatusCode::OK
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_session_reuse_with_client_id() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive, _feed) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let client_id = "test_client_1".to_string();

    // 1. 第一次登录 (带 client_id)
    let res = assert_post!(
        &client,
        format!("{}/api/v1/auth/login", base_url),
        None::<&str>,
        &LoginRequest {
            username: "admin".to_string(),
            password: "test_admin_pwd".to_string(),
            client_id: client_id.clone(),
        },
        StatusCode::OK
    );

    let login_data1: ApiResponse<LoginResponse> = res.json().await?;
    let at1 = login_data1
        .data
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Missing data 1"))?
        .access_token
        .clone();

    // 从 AT 中提取 sid
    let claims1 = okane_api::middleware::auth::verify_jwt(&at1, "YOUR_SUPER_SECRET_KEY")?;
    let sid1 = claims1.sid;

    // 2. 第二次登录 (带相同 client_id)
    let login_req = LoginRequest {
        username: "admin".to_string(),
        password: "test_admin_pwd".to_string(),
        client_id,
    };
    let res = assert_post!(
        &client,
        format!("{}/api/v1/auth/login", base_url),
        None::<&str>,
        &login_req,
        StatusCode::OK
    );

    let login_data2: ApiResponse<LoginResponse> = res.json().await?;
    let at2 = login_data2
        .data
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Missing data 2"))?
        .access_token
        .clone();

    let claims2 = okane_api::middleware::auth::verify_jwt(&at2, "YOUR_SUPER_SECRET_KEY")?;
    let sid2 = claims2.sid;

    assert_eq!(
        sid1, sid2,
        "Session ID must be reused for the same client_id"
    );

    // 3. 第三次登录 (带不同 client_id)
    let client_id_2 = "test_client_2".to_string();
    let res = assert_post!(
        &client,
        format!("{}/api/v1/auth/login", base_url),
        None::<&str>,
        &LoginRequest {
            username: "admin".to_string(),
            password: "test_admin_pwd".to_string(),
            client_id: client_id_2,
        },
        StatusCode::OK
    );

    let login_data3: ApiResponse<LoginResponse> = res.json().await?;
    let at3 = &login_data3
        .data
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Missing data 3"))?
        .access_token;
    let sid3 = okane_api::middleware::auth::verify_jwt(at3, "YOUR_SUPER_SECRET_KEY")?.sid;

    assert!(
        sid1 != sid3,
        "New session must be created for a different client_id"
    );

    Ok(())
}
