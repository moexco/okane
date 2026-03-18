mod common;

use anyhow::Context;
use common::spawn_test_server;
use okane_api::types::{
    AccountSnapshotResponse, ApiResponse, CreateAccountRequest, LoginRequest, LoginResponse,
    NotifyConfigResponse, OrderResponse, StockMetadataResponse, UpdateNotifyConfigRequest,
    UpdateSettingsRequest, WatchlistRequest,
};
use reqwest::StatusCode;
use std::str::FromStr;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_admin_settings_api() -> anyhow::Result<()> {
    let (base_url, store, _tmp, _keepalive, _feed) = spawn_test_server().await?;
    let client = reqwest::Client::new();

    let res = assert_post!(
        &client,
        format!("{}/api/v1/auth/login", base_url),
        None::<&str>,
        &LoginRequest {
            username: "admin".to_string(),
            password: "test_admin_pwd".to_string(),
            client_id: "detailed_test_client".to_string(),
        },
        StatusCode::OK
    );
    let login_data = res
        .json::<ApiResponse<LoginResponse>>()
        .await
        .map_err(|e| anyhow::anyhow!("Parse login: {}", e))?;
    let token = login_data
        .data
        .ok_or_else(|| anyhow::anyhow!("Token null"))?
        .access_token;

    // 1. 更新设置
    assert_put!(
        &client,
        format!("{}/api/v1/admin/settings", base_url),
        Some(&token),
        &UpdateSettingsRequest {
            setting_key: "maintenance_mode".to_string(),
            setting_value: "true".to_string(),
        },
        StatusCode::OK
    );

    // 2. 验证更新（直接验证数据库“落盘”，因为目前尚无 GET /admin/settings 接口）
    let val = store
        .get_setting("maintenance_mode")
        .await?
        .context("Setting not in DB")?;
    assert_eq!(val, "true");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_account_management_api() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive, _feed) = spawn_test_server().await?;
    let client = reqwest::Client::new();

    let res = assert_post!(
        &client,
        format!("{}/api/v1/auth/login", base_url),
        None::<&str>,
        &LoginRequest {
            username: "admin".to_string(),
            password: "test_admin_pwd".to_string(),
            client_id: "detailed_test_client".to_string(),
        },
        StatusCode::OK
    );
    let login_data = res
        .json::<ApiResponse<LoginResponse>>()
        .await
        .map_err(|e| anyhow::anyhow!("Parse login: {}", e))?;
    let token = login_data
        .data
        .ok_or_else(|| anyhow::anyhow!("Token null"))?
        .access_token;

    // 1. 创建账号
    assert_post!(
        &client,
        format!("{}/api/v1/user/account", base_url),
        Some(&token),
        &CreateAccountRequest {
            account_id: "test_detailed_acc".to_string(),
            initial_balance: Some("5000.00".to_string()),
        },
        StatusCode::OK
    );

    // 2. 查询列表 (确保 trader_01 和 test_detailed_acc 都在)
    let res = assert_get!(
        &client,
        format!("{}/api/v1/user/accounts", base_url),
        Some(&token),
        StatusCode::OK
    );
    let accounts = res
        .json::<ApiResponse<Vec<String>>>()
        .await
        .map_err(|e| anyhow::anyhow!("Parse: {}", e))?
        .data
        .ok_or_else(|| anyhow::anyhow!("Data null"))?;
    assert!(accounts.iter().any(|a| a == "trader_01"));
    assert!(accounts.iter().any(|a| a == "test_detailed_acc"));

    // 3. 查询单个 (修正：应查询新创建的 test_detailed_acc 以验证完整流程)
    let res = assert_get!(
        &client,
        format!("{}/api/v1/user/account/{}", base_url, "test_detailed_acc"),
        Some(&token),
        StatusCode::OK
    );
    let acc = res
        .json::<ApiResponse<AccountSnapshotResponse>>()
        .await
        .map_err(|e| anyhow::anyhow!("Parse: {}", e))?
        .data
        .ok_or_else(|| anyhow::anyhow!("Data null"))?;
    assert_eq!(acc.account_id, "test_detailed_acc");
    let avail = rust_decimal::Decimal::from_str(&acc.available_balance)
        .map_err(|e| anyhow::anyhow!("Parse actual: {}", e))?;
    let expected = rust_decimal::Decimal::from_str("5000.00")
        .map_err(|e| anyhow::anyhow!("Parse expected: {}", e))?;
    assert_eq!(avail, expected);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_market_and_watchlist_api() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive, _feed) = spawn_test_server().await?;
    let client = reqwest::Client::new();

    let res = assert_post!(
        &client,
        format!("{}/api/v1/auth/login", base_url),
        None::<&str>,
        &LoginRequest {
            username: "admin".to_string(),
            password: "test_admin_pwd".to_string(),
            client_id: "detailed_test_client".to_string(),
        },
        StatusCode::OK
    );
    let login_data = res
        .json::<ApiResponse<LoginResponse>>()
        .await
        .map_err(|e| anyhow::anyhow!("Parse login: {}", e))?;
    let token = login_data
        .data
        .ok_or_else(|| anyhow::anyhow!("Token null"))?
        .access_token;

    // 1. 搜索股票 (YahooProvider 返回空或模拟数据，具体取决于实现，这里假设能搜到 AAPL)
    let res = assert_get!(
        &client,
        format!("{}/api/v1/market/search?q=AAPL", base_url),
        Some(&token),
        StatusCode::OK
    );
    let _results = res
        .json::<ApiResponse<Vec<StockMetadataResponse>>>()
        .await
        .map_err(|e| anyhow::anyhow!("Parse: {}", e))?
        .data
        .ok_or_else(|| anyhow::anyhow!("Data null"))?;
    // YahooProvider 默认可能返回模拟数据或空，此处我们验证接口连通性

    // 2. 获取 K 线 (使用当前时间附近的范围以匹配 Mock 数据)
    let now = chrono::Utc::now();
    let start =
        (now - chrono::Duration::hours(2)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let end = (now + chrono::Duration::hours(1)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let res = assert_get!(
        &client,
        format!(
            "{}/api/v1/market/candles/AAPL?tf=1m&start={}&end={}",
            base_url, start, end
        ),
        Some(&token),
        StatusCode::OK
    );
    let candles: ApiResponse<Vec<okane_api::types::CandleResponse>> = res.json().await?;
    let data = candles.data.context("data null")?;
    // 校验 K 线内容：不应为空（Mock 数据或者 Yahoo 返回）
    assert!(!data.is_empty(), "Candles should not be empty for AAPL");
    // 校验第一个 K 线的时间范围
    assert!(
        data[0]
            .time
            .parse::<chrono::DateTime<chrono::Utc>>()
            .is_ok()
    );

    // 3. 自选股操作
    assert_post!(
        &client,
        format!("{}/api/v1/user/watchlist", base_url),
        Some(&token),
        &WatchlistRequest {
            symbol: "AAPL".to_string(),
        },
        StatusCode::OK
    );

    let res = assert_get!(
        &client,
        format!("{}/api/v1/user/watchlist", base_url),
        Some(&token),
        StatusCode::OK
    );
    let list = res
        .json::<ApiResponse<Vec<String>>>()
        .await
        .map_err(|e| anyhow::anyhow!("Parse: {}", e))?
        .data
        .ok_or_else(|| anyhow::anyhow!("Data null"))?;
    assert!(list.contains(&"AAPL".to_string()));

    // 4. 从自选股删除
    assert_delete!(
        &client,
        format!("{}/api/v1/user/watchlist/AAPL", base_url),
        Some(&token),
        StatusCode::OK
    );

    // 验证副作用：确认 AAPL 确实从自选列表中消失 (后置断言)
    let res = assert_get!(
        &client,
        format!("{}/api/v1/user/watchlist", base_url),
        Some(&token),
        StatusCode::OK
    );
    let list = res
        .json::<ApiResponse<Vec<String>>>()
        .await
        .map_err(|e| anyhow::anyhow!("Parse: {}", e))?
        .data
        .ok_or_else(|| anyhow::anyhow!("Data null"))?;
    assert!(
        !list.contains(&"AAPL".to_string()),
        "AAPL should be removed from watchlist after DELETE"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_notification_config_api() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive, _feed) = spawn_test_server().await?;
    let client = reqwest::Client::new();

    let res = assert_post!(
        &client,
        format!("{}/api/v1/auth/login", base_url),
        None::<&str>,
        &LoginRequest {
            username: "admin".to_string(),
            password: "test_admin_pwd".to_string(),
            client_id: "detailed_test_client".to_string(),
        },
        StatusCode::OK
    );
    let login_data = res
        .json::<ApiResponse<LoginResponse>>()
        .await
        .map_err(|e| anyhow::anyhow!("Parse login: {}", e))?;
    let token = login_data
        .data
        .ok_or_else(|| anyhow::anyhow!("Token null"))?
        .access_token;

    // 1. 获取 (初始应为 404 NotFound，因为 DB 里还没存)
    assert_get!(
        &client,
        format!("{}/api/v1/user/notify-config", base_url),
        Some(&token),
        StatusCode::NOT_FOUND
    );

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
    assert_put!(
        &client,
        format!("{}/api/v1/user/notify-config", base_url),
        Some(&token),
        &update_req,
        StatusCode::OK
    );

    // 3. 再次验证
    let res = assert_get!(
        &client,
        format!("{}/api/v1/user/notify-config", base_url),
        Some(&token),
        StatusCode::OK
    );
    let config = res
        .json::<ApiResponse<NotifyConfigResponse>>()
        .await
        .map_err(|e| anyhow::anyhow!("Parse: {}", e))?
        .data
        .ok_or_else(|| anyhow::anyhow!("Data null"))?;
    assert_eq!(config.channel, "telegram");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_manual_trade_api() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive, _feed) = spawn_test_server().await?;
    let client = reqwest::Client::new();

    let res = assert_post!(
        &client,
        format!("{}/api/v1/auth/login", base_url),
        None::<&str>,
        &LoginRequest {
            username: "admin".to_string(),
            password: "test_admin_pwd".to_string(),
            client_id: "detailed_test_client".to_string(),
        },
        StatusCode::OK
    );
    let login_data = res
        .json::<ApiResponse<LoginResponse>>()
        .await
        .map_err(|e| anyhow::anyhow!("Parse login: {}", e))?;
    let token = login_data
        .data
        .ok_or_else(|| anyhow::anyhow!("Token null"))?
        .access_token;

    // 准备一个账号
    assert_post!(
        &client,
        format!("{}/api/v1/user/account", base_url),
        Some(&token),
        &CreateAccountRequest {
            account_id: "trade_acc".to_string(),
            initial_balance: Some("100000.00".to_string()),
        },
        StatusCode::OK
    );

    // 1. 下单 (修正：使用前面步骤中新准备的 trade_acc)
    let order_req = serde_json::json!({
        "account_id": "trade_acc",
        "symbol": "AAPL",
        "direction": "Buy",
        "price": "150.00",
        "volume": "10.0"
    });
    let res = assert_post!(
        &client,
        format!("{}/api/v1/user/orders", base_url),
        Some(&token),
        &order_req,
        StatusCode::OK
    );
    let order_id = res
        .json::<ApiResponse<String>>()
        .await
        .map_err(|e| anyhow::anyhow!("Parse: {}", e))?
        .data
        .ok_or_else(|| anyhow::anyhow!("Data null"))?;

    // 2. 获取挂单列表 (修正：查询 trade_acc)
    let res = assert_get!(
        &client,
        format!("{}/api/v1/user/orders?account_id=trade_acc", base_url),
        Some(&token),
        StatusCode::OK
    );
    let orders_page = res
        .json::<ApiResponse<okane_api::types::Page<OrderResponse>>>()
        .await
        .map_err(|e| anyhow::anyhow!("Parse: {}", e))?
        .data
        .ok_or_else(|| anyhow::anyhow!("Data null"))?;
    assert!(orders_page.items.iter().any(|o| o.id == order_id));

    // 3. 撤单
    assert_delete!(
        &client,
        format!("{}/api/v1/user/orders/{}", base_url, order_id),
        Some(&token),
        StatusCode::OK
    );

    // 4. 确认列表中的订单已经消失
    let res = assert_get!(
        &client,
        format!("{}/api/v1/user/orders?account_id=trade_acc", base_url),
        Some(&token),
        StatusCode::OK
    );
    let orders_page = res
        .json::<ApiResponse<okane_api::types::Page<OrderResponse>>>()
        .await
        .map_err(|e| anyhow::anyhow!("Parse: {}", e))?
        .data
        .ok_or_else(|| anyhow::anyhow!("Data null"))?;

    assert!(
        !orders_page.items.iter().any(|o| o.id == order_id),
        "Order should be removed from active list after cancellation"
    );
    Ok(())
}
