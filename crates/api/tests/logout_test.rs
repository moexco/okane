mod common;

use reqwest::StatusCode;
use okane_api::types::{ApiResponse, LoginRequest, LoginResponse};
use common::spawn_test_server;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_auth_logout_revocation() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive, _feed) = spawn_test_server().await?;
    let client = reqwest::Client::new();

    // 1. 登录
    let login_res = client.post(format!("{}/api/v1/auth/login", base_url))
        .json(&LoginRequest {
            username: "admin".to_string(),
            password: "test_admin_pwd".to_string(),
            client_id: "logout_test_client".to_string(),
        })
        .send()
        .await?;
    
    assert_eq!(login_res.status(), StatusCode::OK);
    let login_data: ApiResponse<LoginResponse> = login_res.json().await?;
    let tokens = login_data.data.ok_or_else(|| anyhow::anyhow!("Login data missing"))?;
    let access_token = tokens.access_token;
    let refresh_token = tokens.refresh_token;

    // 2. 验证当前 Token 可用 (访问受保护接口)
    let profile_res = client.get(format!("{}/api/v1/user/accounts", base_url))
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await?;
    assert_eq!(profile_res.status(), StatusCode::OK);

    // 3. 执行登出
    let logout_res = client.post(format!("{}/api/v1/auth/logout", base_url))
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await?;
    assert_eq!(logout_res.status(), StatusCode::OK);

    // 4. 验证 Access Token 立即失效 (即使过期时间还没到)
    let profile_res_after = client.get(format!("{}/api/v1/user/accounts", base_url))
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await?;
    // 应该返回 401 Unauthorized，因为中间件检查 Session 发现已注销
    assert_eq!(profile_res_after.status(), StatusCode::UNAUTHORIZED);

    // 5. 验证 Refresh Token 也失效 (无法轮转)
    let refresh_res = client.post(format!("{}/api/v1/auth/refresh", base_url))
        .header("Authorization", format!("Bearer {}", refresh_token))
        .send()
        .await?;
    assert_eq!(refresh_res.status(), StatusCode::UNAUTHORIZED);

    Ok(())
}
