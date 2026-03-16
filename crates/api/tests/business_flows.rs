mod common;

use reqwest::StatusCode;
use okane_api::types::{
    ApiResponse, ChangePasswordRequest, CreateUserRequest, LoginRequest, LoginResponse,
    StartStrategyRequest, StrategyResponse, CreateAccountRequest,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};

use common::spawn_test_server;
use anyhow::Context;

/// 辅助函数：将 JS 源码编码为 Base64
fn encode_js_source(source: &str) -> String {
    STANDARD.encode(source)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_auth_and_password_flow() -> anyhow::Result<()> {
    let (base_url, store, _tmp, _keepalive, _feed) = spawn_test_server().await?;
    let client = reqwest::Client::new();

    // 显式开启强制改密以验证该流程
    let mut admin = store.get_user("admin").await?.ok_or_else(|| anyhow::anyhow!("Admin missing"))?;
    admin.force_password_change = true;
    store.save_user(&admin).await?;

    // 1. 登录失败
    assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "admin".to_string(),
        password: "wrongpassword".to_string(),
        client_id: "test_client_id".to_string(),
    }, StatusCode::UNAUTHORIZED);

    // 2. 登陆成功
    let res = assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "admin".to_string(),
        password: "test_admin_pwd".to_string(),
        client_id: "test_client_id".to_string(),
    }, StatusCode::OK);
    let login_data: ApiResponse<LoginResponse> = res.json().await.map_err(|e| anyhow::anyhow!("Failed to parse login response: {}", e))?;
    let admin_token = login_data.data.ok_or_else(|| anyhow::anyhow!("Login response missing data"))?.access_token;

    // 3. 强制修改密码锁定检查
    assert_get!(&client, format!("{}/api/v1/user/strategies", base_url), Some(&admin_token), StatusCode::FORBIDDEN);

    // 4. 修改密码
    assert_post!(&client, format!("{}/api/v1/auth/change_password", base_url), Some(&admin_token), &ChangePasswordRequest {
        old_password: "test_admin_pwd".to_string(),
        new_password: "new_secure_password".to_string(),
    }, StatusCode::OK);

    // 5. 使用新密码登录
    assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "admin".to_string(),
        password: "new_secure_password".to_string(),
        client_id: "test_client_id".to_string(),
    }, StatusCode::OK);

    // 6. 验证旧密码已失效 (后置副作用断言)
    let res = client.post(format!("{}/api/v1/auth/login", base_url))
        .json(&LoginRequest {
            username: "admin".to_string(),
            password: "test_admin_pwd".to_string(),
            client_id: "test_client_id".to_string(),
        })
        .send().await?;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "Old password should be invalid after successful change");

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_admin_user_management() -> anyhow::Result<()> {
    let (base_url, store, _tmp, _keepalive, _feed) = spawn_test_server().await?;
    let client = reqwest::Client::new();

    // 先登录 (跳过强制改密逻辑，测试环境直接用 admin)
    let res = assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "admin".to_string(),
        password: "test_admin_pwd".to_string(),
        client_id: "test_client_id".to_string(),
    }, StatusCode::OK);
    let admin_token = res.json::<ApiResponse<LoginResponse>>().await.map_err(|e| anyhow::anyhow!(e))?.data.ok_or_else(|| anyhow::anyhow!("Admin token null"))?.access_token;

    // 创建新用户
    assert_post!(&client, format!("{}/api/v1/admin/users", base_url), Some(&admin_token), &CreateUserRequest {
        id: "trader_01".to_string(),
        name: "Trader One".to_string(),
        password: "trader_password".to_string(),
        role: "Standard".to_string(),
    }, StatusCode::OK);

    // 验证用户已存在（通过 Store 验证“落盘”）
    let saved_user = store.get_user("trader_01").await?.context("User not found in store")?;
    assert_eq!(saved_user.name, "Trader One");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_user_account_and_strategy_deployment() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive, _feed) = spawn_test_server().await?;
    let client = reqwest::Client::new();

    // 1. 创建 Trader (Admin 权限)
    let res = assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "admin".to_string(),
        password: "test_admin_pwd".to_string(),
        client_id: "test_client_id".to_string(),
    }, StatusCode::OK);
    let admin_token = res.json::<ApiResponse<LoginResponse>>().await.map_err(|e| anyhow::anyhow!(e))?.data.ok_or_else(|| anyhow::anyhow!("Admin data null"))?.access_token;

    assert_post!(&client, format!("{}/api/v1/admin/users", base_url), Some(&admin_token), &CreateUserRequest {
        id: "trader_02".to_string(),
        name: "Trader Two".to_string(),
        password: "trader_password".to_string(),
        role: "Standard".to_string(),
    }, StatusCode::OK);

    // 2. Trader 登录并改密
    let res = assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "trader_02".to_string(),
        password: "trader_password".to_string(),
        client_id: "test_client_id".to_string(),
    }, StatusCode::OK);
    let trader_token = res.json::<ApiResponse<LoginResponse>>().await.map_err(|e| anyhow::anyhow!(e))?.data.ok_or_else(|| anyhow::anyhow!("Trader data null"))?.access_token;

    assert_post!(&client, format!("{}/api/v1/auth/change_password", base_url), Some(&trader_token), &ChangePasswordRequest {
        old_password: "trader_password".to_string(),
        new_password: "trader_new_pwd".to_string(),
    }, StatusCode::OK);

    // 改密后原 Token 撤销，需要重新登录获取新 Token
    let res = assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "trader_02".to_string(),
        password: "trader_new_pwd".to_string(),
        client_id: "test_client_id".to_string(),
    }, StatusCode::OK);
    let trader_token = res.json::<ApiResponse<LoginResponse>>().await.map_err(|e| anyhow::anyhow!(e))?.data.ok_or_else(|| anyhow::anyhow!("Trader data null"))?.access_token;

    // 3. 注册金融账号
    assert_post!(&client, format!("{}/api/v1/user/account", base_url), Some(&trader_token), &CreateAccountRequest {
        account_id: "acc_trader_02".to_string(),
        initial_balance: Some("100000.00".to_string()),
    }, StatusCode::OK);

    // 4. 部署策略 (源码透明化)
    let js_code = r#"
        function onInit() {
            host.log("Strategy initialized");
        }
        function onCandle(input) {
            var candle = JSON.parse(input);
            host.buy('AAPL', candle.close.toString(), "10.0");
        }
    "#;
    let deploy_res = assert_post!(&client, format!("{}/api/v1/user/strategies", base_url), Some(&trader_token), &StartStrategyRequest {
        symbol: "AAPL".to_string(),
        account_id: "acc_trader_02".to_string(),
        timeframe: "1m".to_string(),
        engine_type: "JavaScript".to_string(),
        source_base64: encode_js_source(js_code),
    }, StatusCode::OK);
    
    let strategy_id = deploy_res.json::<ApiResponse<String>>().await.map_err(|e| anyhow::anyhow!(e))?.data.ok_or_else(|| anyhow::anyhow!("Strategy ID null"))?;

    // 5. 验证列表
    let list_res = assert_get!(&client, format!("{}/api/v1/user/strategies", base_url), Some(&trader_token), StatusCode::OK);
    let strategies_page = list_res.json::<ApiResponse<okane_api::types::Page<StrategyResponse>>>().await.map_err(|e| anyhow::anyhow!(e))?.data.ok_or_else(|| anyhow::anyhow!("Strategies null"))?;
    assert!(strategies_page.items.iter().any(|s| s.id == strategy_id));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_strategy_backtest() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive, _feed) = spawn_test_server().await?;
    let client = reqwest::Client::new();

    // 获取 Admin Token 并创建用户 (为了保持测试独立，重复 setup 是必要的)
    let res = assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "admin".to_string(),
        password: "test_admin_pwd".to_string(),
        client_id: "test_client_id".to_string(),
    }, StatusCode::OK);
    let admin_data = res.json::<ApiResponse<LoginResponse>>().await.map_err(|e| anyhow::anyhow!(e))?;
    let admin_token = admin_data.data.ok_or_else(|| anyhow::anyhow!("Admin data null"))?.access_token;

    assert_post!(&client, format!("{}/api/v1/admin/users", base_url), Some(&admin_token), &CreateUserRequest {
        id: "bt_user".to_string(),
        name: "BT User".to_string(),
        password: "password".to_string(),
        role: "Standard".to_string(),
    }, StatusCode::OK);

    let res = assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "bt_user".to_string(),
        password: "password".to_string(),
        client_id: "test_client_id".to_string(),
    }, StatusCode::OK);
    let user_data = res.json::<ApiResponse<LoginResponse>>().await.map_err(|e| anyhow::anyhow!(e))?;
    let token = user_data.data.ok_or_else(|| anyhow::anyhow!("User data null"))?.access_token;

    // 必须改密码
    assert_post!(&client, format!("{}/api/v1/auth/change_password", base_url), Some(&token), &ChangePasswordRequest {
        old_password: "password".to_string(),
        new_password: "new_password".to_string(),
    }, StatusCode::OK);

    // 重新登录获取新 Token (或继续使用旧的，但为了严谨通常用新的)
    let res = assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "bt_user".to_string(),
        password: "new_password".to_string(),
        client_id: "test_client_id".to_string(),
    }, StatusCode::OK);
    let token_data = res.json::<ApiResponse<LoginResponse>>().await.map_err(|e| anyhow::anyhow!(e))?;
    let token = token_data.data.ok_or_else(|| anyhow::anyhow!("Token data null"))?.access_token;

    // 回测逻辑
    let js_code = "function onInit() {} function onCandle(input) { host.buy('AAPL', '150.0', '10'); }";
    let end_time = chrono::Utc::now();
    let start_time = end_time - chrono::Duration::days(1);
    
    let res = assert_post!(&client, format!("{}/api/v1/user/backtest", base_url), Some(&token), &serde_json::json!({
        "symbol": "AAPL",
        "timeframe": "1d",
        "start": start_time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "end": end_time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "initial_balance": "10000.00",
        "engine_type": "JavaScript",
        "source_base64": encode_js_source(js_code)
    }), StatusCode::OK);
    
    let bt_data = res.json::<ApiResponse<okane_api::types::BacktestResponse>>().await.map_err(|e| anyhow::anyhow!(e))?;
    let result = bt_data.data.ok_or_else(|| anyhow::anyhow!("Backtest data null"))?;
    assert!(result.final_snapshot.account_id.starts_with("backtest_"));
    Ok(())
}
