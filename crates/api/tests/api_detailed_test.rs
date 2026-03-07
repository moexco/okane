mod common;

use reqwest::StatusCode;
use okane_api::types::{
    ApiResponse, LoginRequest, LoginResponse, CreateAccountRequest, AccountSnapshotResponse,
    StockMetadataResponse, NotifyConfigResponse, UpdateNotifyConfigRequest, OrderResponse,
};
use okane_api::routes::admin::UpdateSettingsRequest;
use okane_api::routes::watchlist::WatchlistRequest;
use common::spawn_test_server;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_admin_settings_api() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();

    let res = assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "admin".to_string(),
        password: "test_admin_pwd".to_string(),
    }, StatusCode::OK);
    let login_data = res.json::<ApiResponse<LoginResponse>>().await.map_err(|e| anyhow::anyhow!("Parse login: {}", e))?;
    let token = login_data.data.ok_or_else(|| anyhow::anyhow!("Token null"))?.token;

    // 1. 更新设置
    assert_put!(&client, format!("{}/api/v1/admin/settings", base_url), Some(&token), &UpdateSettingsRequest {
        setting_key: "maintenance_mode".to_string(),
        setting_value: "true".to_string(),
    }, StatusCode::OK);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_account_management_api() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();

    let res = assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "admin".to_string(),
        password: "test_admin_pwd".to_string(),
    }, StatusCode::OK);
    let login_data = res.json::<ApiResponse<LoginResponse>>().await.map_err(|e| anyhow::anyhow!("Parse login: {}", e))?;
    let token = login_data.data.ok_or_else(|| anyhow::anyhow!("Token null"))?.token;

    // 1. 创建账号
    assert_post!(&client, format!("{}/api/v1/user/account", base_url), Some(&token), &CreateAccountRequest {
        account_id: "test_detailed_acc".to_string(),
        initial_balance: Some("5000.00".to_string()),
    }, StatusCode::OK);

    // 2. 查询列表 (确保 trader_01 和 test_detailed_acc 都在)
    let res = assert_get!(&client, format!("{}/api/v1/user/accounts", base_url), Some(&token), StatusCode::OK);
    let accounts = res.json::<ApiResponse<Vec<String>>>().await.map_err(|e| anyhow::anyhow!("Parse: {}", e))?.data.ok_or_else(|| anyhow::anyhow!("Data null"))?;
    assert!(accounts.iter().any(|a| a == "trader_01"));
    assert!(accounts.iter().any(|a| a == "test_detailed_acc"));

    // 3. 查询单个 (使用 trader_01, 因为它在 AccountManager 中已激活)
    let res = assert_get!(&client, format!("{}/api/v1/user/account/{}", base_url, "trader_01"), Some(&token), StatusCode::OK);
    let acc = res.json::<ApiResponse<AccountSnapshotResponse>>().await.map_err(|e| anyhow::anyhow!("Parse: {}", e))?.data.ok_or_else(|| anyhow::anyhow!("Data null"))?;
    assert_eq!(acc.account_id, "trader_01");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_market_and_watchlist_api() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();

    let res = assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "admin".to_string(),
        password: "test_admin_pwd".to_string(),
    }, StatusCode::OK);
    let login_data = res.json::<ApiResponse<LoginResponse>>().await.map_err(|e| anyhow::anyhow!("Parse login: {}", e))?;
    let token = login_data.data.ok_or_else(|| anyhow::anyhow!("Token null"))?.token;

    // 1. 搜索股票 (YahooProvider 返回空或模拟数据，具体取决于实现，这里假设能搜到 AAPL)
    let res = assert_get!(&client, format!("{}/api/v1/market/search?q=AAPL", base_url), Some(&token), StatusCode::OK);
    let _results = res.json::<ApiResponse<Vec<StockMetadataResponse>>>().await.map_err(|e| anyhow::anyhow!("Parse: {}", e))?.data.ok_or_else(|| anyhow::anyhow!("Data null"))?;
    // YahooProvider 默认可能返回模拟数据或空，此处我们验证接口连通性

    // 2. 获取 K 线 (需要 tf, start, end 参数)
    let start = "2026-03-01T00:00:00Z";
    let end = "2026-03-05T00:00:00Z";
    let _res = assert_get!(&client, format!("{}/api/v1/market/candles/AAPL?tf=1m&start={}&end={}", base_url, start, end), Some(&token), StatusCode::OK);

    // 3. 自选股操作
    assert_post!(&client, format!("{}/api/v1/user/watchlist", base_url), Some(&token), &WatchlistRequest {
        symbol: "AAPL".to_string(),
    }, StatusCode::OK);

    let res = assert_get!(&client, format!("{}/api/v1/user/watchlist", base_url), Some(&token), StatusCode::OK);
    let list = res.json::<ApiResponse<Vec<String>>>().await.map_err(|e| anyhow::anyhow!("Parse: {}", e))?.data.ok_or_else(|| anyhow::anyhow!("Data null"))?;
    assert!(list.contains(&"AAPL".to_string()));

    assert_delete!(&client, format!("{}/api/v1/user/watchlist/AAPL", base_url), Some(&token), StatusCode::OK);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_notification_config_api() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();

    let res = assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "admin".to_string(),
        password: "test_admin_pwd".to_string(),
    }, StatusCode::OK);
    let login_data = res.json::<ApiResponse<LoginResponse>>().await.map_err(|e| anyhow::anyhow!("Parse login: {}", e))?;
    let token = login_data.data.ok_or_else(|| anyhow::anyhow!("Token null"))?.token;

    // 1. 获取 (初始应为 404 NotFound，因为 DB 里还没存)
    assert_get!(&client, format!("{}/api/v1/user/notify-config", base_url), Some(&token), StatusCode::NOT_FOUND);

    // 2. 更新为 telegram
    let update_req = UpdateNotifyConfigRequest {
        channel: "telegram".to_string(),
        telegram: okane_api::types::TelegramConfig {
            bot_token: "123:ABC".to_string(),
            chat_id: "456".to_string(),
        },
        email: okane_api::types::EmailConfig {
            smtp_host: "".to_string(),
            smtp_user: "".to_string(),
            smtp_pass: "".to_string(),
            from: "".to_string(),
            to: "".to_string(),
        },
    };
    assert_put!(&client, format!("{}/api/v1/user/notify-config", base_url), Some(&token), &update_req, StatusCode::OK);

    // 3. 再次验证
    let res = assert_get!(&client, format!("{}/api/v1/user/notify-config", base_url), Some(&token), StatusCode::OK);
    let config = res.json::<ApiResponse<NotifyConfigResponse>>().await.map_err(|e| anyhow::anyhow!("Parse: {}", e))?.data.ok_or_else(|| anyhow::anyhow!("Data null"))?;
    assert_eq!(config.channel, "telegram");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_manual_trade_api() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();

    let res = assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "admin".to_string(),
        password: "test_admin_pwd".to_string(),
    }, StatusCode::OK);
    let login_data = res.json::<ApiResponse<LoginResponse>>().await.map_err(|e| anyhow::anyhow!("Parse login: {}", e))?;
    let token = login_data.data.ok_or_else(|| anyhow::anyhow!("Token null"))?.token;

    // 准备一个账号
    assert_post!(&client, format!("{}/api/v1/user/account", base_url), Some(&token), &CreateAccountRequest {
        account_id: "trade_acc".to_string(),
        initial_balance: Some("100000.00".to_string()),
    }, StatusCode::OK);

    // 1. 下单 (使用已在 AccountManager 激活的 trader_01)
    let order_req = serde_json::json!({
        "account_id": "trader_01",
        "symbol": "AAPL",
        "direction": "Buy",
        "price": "150.00",
        "volume": "10.0"
    });
    let res = assert_post!(&client, format!("{}/api/v1/user/orders", base_url), Some(&token), &order_req, StatusCode::OK);
    let order_id = res.json::<ApiResponse<String>>().await.map_err(|e| anyhow::anyhow!("Parse: {}", e))?.data.ok_or_else(|| anyhow::anyhow!("Data null"))?;

    // 2. 获取挂单列表
    let res = assert_get!(&client, format!("{}/api/v1/user/orders?account_id=trader_01", base_url), Some(&token), StatusCode::OK);
    let orders = res.json::<ApiResponse<Vec<OrderResponse>>>().await.map_err(|e| anyhow::anyhow!("Parse: {}", e))?.data.ok_or_else(|| anyhow::anyhow!("Data null"))?;
    assert!(orders.iter().any(|o| o.id == order_id));

    // 3. 撤单
    assert_delete!(&client, format!("{}/api/v1/user/orders/{}", base_url, order_id), Some(&token), StatusCode::OK);

    // 4. 确认列表为空 (或状态为 Canceled/Rejected)
    let res = assert_get!(&client, format!("{}/api/v1/user/orders?account_id=trader_01", base_url), Some(&token), StatusCode::OK);
    let orders = res.json::<ApiResponse<Vec<OrderResponse>>>().await.map_err(|e| anyhow::anyhow!("Parse: {}", e))?.data.ok_or_else(|| anyhow::anyhow!("Data null"))?;
    // 如果订单被立即处理，它可能变状态。本地 Mock 撮合可能会立即撮合。
    assert!(orders.iter().filter(|o| o.id == order_id).all(|o| o.status == "Canceled" || o.status == "Rejected" || o.status == "Filled"));
    Ok(())
}
