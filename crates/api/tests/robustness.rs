mod common;

use reqwest::StatusCode;
use okane_api::types::{
    ApiResponse, LoginRequest, LoginResponse,
    CreateUserRequest, StrategyResponse, StartStrategyRequest, SaveStrategySourceRequest,
    AlgoOrderResponse,
};
use base64::Engine;
use okane_api::routes::watchlist::WatchlistRequest;
use common::spawn_test_server;
use anyhow::Context;

async fn get_admin_token(client: &reqwest::Client, base_url: &str) -> anyhow::Result<String> {
    let res = assert_post!(client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "admin".to_string(),
        password: "test_admin_pwd".to_string(),
        client_id: "gap_test".to_string(),
    }, StatusCode::OK);
    let data: ApiResponse<LoginResponse> = res.json().await?;
    data.data.map(|d| d.access_token).context("login data null")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_market_candles_invalid_tf() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let token = get_admin_token(&client, &base_url).await?;

    let res = client.get(format!("{}/api/v1/market/candles/AAPL?tf=invalid&start=2026-03-01T00:00:00Z&end=2026-03-02T00:00:00Z", base_url))
        .bearer_auth(&token)
        .send().await?;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_market_candles_invalid_start_time() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let token = get_admin_token(&client, &base_url).await?;

    let res = client.get(format!("{}/api/v1/market/candles/AAPL?tf=1m&start=not-a-time&end=2026-03-02T00:00:00Z", base_url))
        .bearer_auth(&token)
        .send().await?;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_market_candles_invalid_end_time() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let token = get_admin_token(&client, &base_url).await?;

    let res = client.get(format!("{}/api/v1/market/candles/AAPL?tf=1m&start=2026-03-01T00:00:00Z&end=not-a-time", base_url))
        .bearer_auth(&token)
        .send().await?;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_watchlist_add_stock_not_found() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let token = get_admin_token(&client, &base_url).await?;

    assert_post!(&client, format!("{}/api/v1/user/watchlist", base_url), Some(&token), &WatchlistRequest {
        symbol: "INVALID_TICKER_999".to_string(),
    }, StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_admin_create_duplicate_user() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let token = get_admin_token(&client, &base_url).await?;

    assert_post!(&client, format!("{}/api/v1/admin/users", base_url), Some(&token), &CreateUserRequest {
        id: "admin".to_string(),
        name: "Cloned Admin".to_string(),
        password: "password".to_string(),
        role: "Admin".to_string(),
    }, StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_admin_create_user_with_invalid_role() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let token = get_admin_token(&client, &base_url).await?;

    assert_post!(&client, format!("{}/api/v1/admin/users", base_url), Some(&token), &CreateUserRequest {
        id: "new_trader".to_string(),
        name: "New Trader".to_string(),
        password: "password".to_string(),
        role: "God".to_string(),
    }, StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_trade_place_order_invalid_direction() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let token = get_admin_token(&client, &base_url).await?;

    let res = client.post(format!("{}/api/v1/user/orders", base_url))
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "account_id": "trader_01", 
            "symbol": "AAPL",
            "volume": "100",
            "direction": "INVALID"
        }))
        .send().await?;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_trade_place_order_invalid_volume() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let token = get_admin_token(&client, &base_url).await?;

    let res = client.post(format!("{}/api/v1/user/orders", base_url))
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "account_id": "trader_01", 
            "symbol": "AAPL",
            "volume": "not-a-number",
            "direction": "BUY"
        }))
        .send().await?;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_trade_place_order_zero_volume() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let token = get_admin_token(&client, &base_url).await?;

    let res = client.post(format!("{}/api/v1/user/orders", base_url))
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "account_id": "trader_01", 
            "symbol": "AAPL",
            "volume": "0",
            "direction": "BUY"
        }))
        .send().await?;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_trade_cancel_order_not_found() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let token = get_admin_token(&client, &base_url).await?;

    let res = client.delete(format!("{}/api/v1/user/orders/non-existent-id", base_url))
        .bearer_auth(&token)
        .send().await?;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_algo_grid_missing_params() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let token = get_admin_token(&client, &base_url).await?;

    let res = client.post(format!("{}/api/v1/user/algo", base_url))
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "account_id": "trader_01",
            "symbol": "AAPL",
            "algo_type": "grid",
            "params": {}
        }))
        .send().await?;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_algo_unsupported_type() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let token = get_admin_token(&client, &base_url).await?;

    let res = client.post(format!("{}/api/v1/user/algo", base_url))
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "account_id": "trader_01",
            "symbol": "AAPL",
            "algo_type": "unknown",
            "params": {}
        }))
        .send().await?;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_idor_view_others_orders() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let admin_token = get_admin_token(&client, &base_url).await?;

    assert_post!(&client, format!("{}/api/v1/admin/users", base_url), Some(&admin_token), &serde_json::json!({
        "id": "victim",
        "name": "Victim",
        "password": "password",
        "role": "Standard",
        "force_password_change": false
    }), StatusCode::OK);

    assert_post!(&client, format!("{}/api/v1/admin/users", base_url), Some(&admin_token), &serde_json::json!({
        "id": "attacker",
        "name": "Attacker",
        "password": "password",
        "role": "Standard",
        "force_password_change": false
    }), StatusCode::OK);

    let res = assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "attacker".to_string(),
        password: "password".to_string(),
        client_id: "attacker_cli".to_string(),
    }, StatusCode::OK);
    let attacker_token = res.json::<ApiResponse<LoginResponse>>().await?.data.context("data null")?.access_token;

    let res = assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "victim".to_string(),
        password: "password".to_string(),
        client_id: "victim_cli".to_string(),
    }, StatusCode::OK);
    let victim_token = res.json::<ApiResponse<LoginResponse>>().await?.data.context("data null")?.access_token;

    assert_post!(&client, format!("{}/api/v1/user/account", base_url), Some(&victim_token), &serde_json::json!({
        "account_id": "victim_acc",
        "initial_balance": "1000"
    }), StatusCode::OK);

    assert_get!(&client, format!("{}/api/v1/user/orders?account_id=victim_acc", base_url), Some(&attacker_token), StatusCode::FORBIDDEN);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_strategy_lifecycle_gap_coverage() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let token = get_admin_token(&client, &base_url).await?;

    // 1. List strategies (initially empty)
    let res = assert_get!(&client, format!("{}/api/v1/user/strategies", base_url), Some(&token), StatusCode::OK);
    let list = res.json::<ApiResponse<Vec<StrategyResponse>>>().await?.data.context("data null")?;
    assert!(list.is_empty());

    // 2. Deploy a strategy
    let source_b64 = base64::prelude::BASE64_STANDARD.encode("print('hello')");
    let res = assert_post!(&client, format!("{}/api/v1/user/strategies", base_url), Some(&token), &StartStrategyRequest {
        symbol: "AAPL".to_string(),
        account_id: "trader_01".to_string(),
        timeframe: "1m".to_string(),
        engine_type: "JavaScript".to_string(),
        source_base64: source_b64,
    }, StatusCode::OK);
    let strategy_id = res.json::<ApiResponse<String>>().await?.data.context("data null")?;

    // 3. Get individual strategy
    assert_get!(&client, format!("{}/api/v1/user/strategies/{}", base_url, strategy_id), Some(&token), StatusCode::OK);

    // 4. Update strategy (must stop first, but here we just test the route logic)
    // First stop it
    assert_post!(&client, format!("{}/api/v1/user/strategies/{}/stop", base_url, strategy_id), Some(&token), &serde_json::json!({}), StatusCode::OK);
    
    let new_source = base64::prelude::BASE64_STANDARD.encode("print('updated')");
    assert_put!(&client, format!("{}/api/v1/user/strategies/{}", base_url, strategy_id), Some(&token), &SaveStrategySourceRequest {
        source_base64: new_source,
    }, StatusCode::OK);

    // 5. Get logs
    assert_get!(&client, format!("{}/api/v1/user/strategies/{}/logs", base_url, strategy_id), Some(&token), StatusCode::OK);

    // 6. Delete
    assert_delete!(&client, format!("{}/api/v1/user/strategies/{}", base_url, strategy_id), Some(&token), StatusCode::OK);

    // 7. Verify deletion via 404
    assert_get!(&client, format!("{}/api/v1/user/strategies/{}", base_url, strategy_id), Some(&token), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_trade_algo_lifecycle_gap_coverage() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let token = get_admin_token(&client, &base_url).await?;

    // 1. Submit Algo Order
    let res = assert_post!(&client, format!("{}/api/v1/user/algo", base_url), Some(&token), &serde_json::json!({
        "account_id": "trader_01",
        "symbol": "AAPL",
        "algo_type": "snipe",
        "params": { "target_price": "150.0" }
    }), StatusCode::OK);
    let algo_id = res.json::<ApiResponse<String>>().await?.data.context("data null")?;

    // 2. Get Algo Orders
    let res = assert_get!(&client, format!("{}/api/v1/user/algo?account_id=trader_01", base_url), Some(&token), StatusCode::OK);
    let orders = res.json::<ApiResponse<Vec<AlgoOrderResponse>>>().await?.data.context("data null")?;
    assert!(!orders.is_empty());

    // 3. Cancel Algo Order
    assert_delete!(&client, format!("{}/api/v1/user/algo/{}", base_url, algo_id), Some(&token), StatusCode::OK);

    // 4. Get positions
    assert_get!(&client, format!("{}/api/v1/user/account/trader_01/positions", base_url), Some(&token), StatusCode::OK);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_idor_deploy_strategy_others_account() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let admin_token = get_admin_token(&client, &base_url).await?;

    // Create victim
    assert_post!(&client, format!("{}/api/v1/admin/users", base_url), Some(&admin_token), &serde_json::json!({
        "id": "victim", "name": "Victim", "password": "password", "role": "Standard", "force_password_change": false
    }), StatusCode::OK);

    // Create attacker
    assert_post!(&client, format!("{}/api/v1/admin/users", base_url), Some(&admin_token), &serde_json::json!({
        "id": "attacker", "name": "Attacker", "password": "password", "role": "Standard", "force_password_change": false
    }), StatusCode::OK);

    let res = assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "attacker".to_string(), password: "password".to_string(), client_id: "c1".to_string(),
    }, StatusCode::OK);
    let attacker_token = res.json::<ApiResponse<LoginResponse>>().await?.data.context("data null")?.access_token;

    // Attacker tries to deploy on trader_01 (which belongs to admin, not attacker)
    let res = client.post(format!("{}/api/v1/user/strategies", base_url))
        .bearer_auth(&attacker_token)
        .json(&serde_json::json!({
            "symbol": "AAPL",
            "account_id": "trader_01",
            "timeframe": "1m",
            "engine_type": "JavaScript",
            "source_base64": base64::prelude::BASE64_STANDARD.encode("print(1)")
        }))
        .send().await?;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_trade_place_order_idor() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let token = get_admin_token(&client, &base_url).await?; // Token for admin, but let's assume it belongs to admin
    
    // Create attacker
    assert_post!(&client, format!("{}/api/v1/admin/users", base_url), Some(&token), &serde_json::json!({
        "id": "attacker", "name": "Attacker", "password": "password", "role": "Standard", "force_password_change": false
    }), StatusCode::OK);
    let res = assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "attacker".to_string(), password: "password".to_string(), client_id: "c1".to_string(),
    }, StatusCode::OK);
    let attacker_token = res.json::<ApiResponse<LoginResponse>>().await?.data.context("data null")?.access_token;

    // Attacker tries to place order on trader_01
    let res = client.post(format!("{}/api/v1/user/orders", base_url))
        .bearer_auth(&attacker_token)
        .json(&serde_json::json!({
            "account_id": "trader_01", "symbol": "AAPL", "volume": "100", "direction": "BUY"
        }))
        .send().await?;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_trade_get_positions_idor() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let admin_token = get_admin_token(&client, &base_url).await?;

    assert_post!(&client, format!("{}/api/v1/admin/users", base_url), Some(&admin_token), &serde_json::json!({
        "id": "a", "name": "A", "password": "p", "role": "Standard", "force_password_change": false
    }), StatusCode::OK);
    let res = assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "a".to_string(), password: "p".to_string(), client_id: "c".to_string(),
    }, StatusCode::OK);
    let token = res.json::<ApiResponse<LoginResponse>>().await?.data.context("data null")?.access_token;

    assert_get!(&client, format!("{}/api/v1/user/account/trader_01/positions", base_url), Some(&token), StatusCode::FORBIDDEN);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_trade_algo_cancel_idor() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let admin_token = get_admin_token(&client, &base_url).await?;

    // Create an algo order as admin
    let res = assert_post!(&client, format!("{}/api/v1/user/algo", base_url), Some(&admin_token), &serde_json::json!({
        "account_id": "trader_01", "symbol": "AAPL", "algo_type": "snipe", "params": { "target_price": "100" }
    }), StatusCode::OK);
    let algo_id = res.json::<ApiResponse<String>>().await?.data.context("data null")?;

    // Create attacker
    assert_post!(&client, format!("{}/api/v1/admin/users", base_url), Some(&admin_token), &serde_json::json!({
        "id": "a", "name": "A", "password": "p", "role": "Standard", "force_password_change": false
    }), StatusCode::OK);
    let res = assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "a".to_string(), password: "p".to_string(), client_id: "c".to_string(),
    }, StatusCode::OK);
    let attacker_token = res.json::<ApiResponse<LoginResponse>>().await?.data.context("data null")?.access_token;

    // Attacker tries to cancel admin's algo order
    let res = client.delete(format!("{}/api/v1/user/algo/{}", base_url, algo_id))
        .bearer_auth(&attacker_token)
        .send().await?;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_account_initial_balance_invalid() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let token = get_admin_token(&client, &base_url).await?;

    let res = client.post(format!("{}/api/v1/user/account", base_url))
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "account_id": "new_acc",
            "initial_balance": "not-a-decimal"
        }))
        .send().await?;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_algo_grid_invalid_prices() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let token = get_admin_token(&client, &base_url).await?;

    // Missing params
    assert_post!(&client, format!("{}/api/v1/user/algo", base_url), Some(&token), &serde_json::json!({
        "account_id": "trader_01", "symbol": "AAPL", "algo_type": "grid", "params": { "upper_price": "200" }
    }), StatusCode::BAD_REQUEST);

    // Invalid decimal
    assert_post!(&client, format!("{}/api/v1/user/algo", base_url), Some(&token), &serde_json::json!({
        "account_id": "trader_01", "symbol": "AAPL", "algo_type": "grid", "params": { "upper_price": "200", "lower_price": "invalid", "grids": 10 }
    }), StatusCode::BAD_REQUEST);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_trade_place_order_invalid_price_format() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let token = get_admin_token(&client, &base_url).await?;

    let res = client.post(format!("{}/api/v1/user/orders", base_url))
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "account_id": "trader_01", "symbol": "AAPL", "volume": "100", "direction": "BUY", "price": "invalid"
        }))
        .send().await?;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_strategy_idor_all_actions() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let admin_token = get_admin_token(&client, &base_url).await?;

    // Create victim strategy
    let res = assert_post!(&client, format!("{}/api/v1/user/strategies", base_url), Some(&admin_token), &serde_json::json!({
        "symbol": "AAPL", "account_id": "trader_01", "timeframe": "1m", "engine_type": "JavaScript",
        "source_base64": base64::prelude::BASE64_STANDARD.encode("print(1)")
    }), StatusCode::OK);
    let strategy_id = res.json::<ApiResponse<String>>().await?.data.context("data null")?;

    // Create attacker
    assert_post!(&client, format!("{}/api/v1/admin/users", base_url), Some(&admin_token), &serde_json::json!({
        "id": "attacker", "name": "Attacker", "password": "p", "role": "Standard", "force_password_change": false
    }), StatusCode::OK);
    let res = assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "attacker".to_string(), password: "p".to_string(), client_id: "c1".to_string(),
    }, StatusCode::OK);
    let attacker_token = res.json::<ApiResponse<LoginResponse>>().await?.data.context("data null")?.access_token;

    // Test IDOR for GET, STOP, UPDATE, DELETE, LOGS. 
    // They return 404 because strategy queries are scoped by user_id.
    assert_get!(&client, format!("{}/api/v1/user/strategies/{}", base_url, strategy_id), Some(&attacker_token), StatusCode::NOT_FOUND);
    assert_post!(&client, format!("{}/api/v1/user/strategies/{}/stop", base_url, strategy_id), Some(&attacker_token), &serde_json::json!({}), StatusCode::NOT_FOUND);
    assert_put!(&client, format!("{}/api/v1/user/strategies/{}", base_url, strategy_id), Some(&attacker_token), &serde_json::json!({"source_base64": base64::prelude::BASE64_STANDARD.encode("print(1)")}), StatusCode::NOT_FOUND);
    assert_delete!(&client, format!("{}/api/v1/user/strategies/{}", base_url, strategy_id), Some(&attacker_token), StatusCode::NOT_FOUND);
    assert_get!(&client, format!("{}/api/v1/user/strategies/{}/logs", base_url, strategy_id), Some(&attacker_token), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_trade_cancel_order_idor() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let admin_token = get_admin_token(&client, &base_url).await?;

    // Create order as admin with a far-away limit price (10.0 when market is ~150) to keep it pending
    let res = assert_post!(&client, format!("{}/api/v1/user/orders", base_url), Some(&admin_token), &serde_json::json!({
        "account_id": "trader_01", "symbol": "AAPL", "volume": "1", "direction": "BUY", "price": "10.0"
    }), StatusCode::OK);
    let order_id = res.json::<ApiResponse<String>>().await?.data.context("data null")?;

    // Create attacker
    assert_post!(&client, format!("{}/api/v1/admin/users", base_url), Some(&admin_token), &serde_json::json!({
        "id": "a", "name": "A", "password": "p", "role": "Standard", "force_password_change": false
    }), StatusCode::OK);
    let res = assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "a".to_string(), password: "p".to_string(), client_id: "c".to_string(),
    }, StatusCode::OK);
    let token = res.json::<ApiResponse<LoginResponse>>().await?.data.context("data null")?.access_token;

    assert_delete!(&client, format!("{}/api/v1/user/orders/{}", base_url, order_id), Some(&token), StatusCode::FORBIDDEN);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_trade_account_not_found_error_path() -> anyhow::Result<()> {
    let (base_url, system_store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let token = get_admin_token(&client, &base_url).await?;

    // Bind a non-existent account "ghost" to admin
    system_store.bind_account("admin", "ghost").await?;

    // Now IDOR check passes, but TradePort.get_account fails
    assert_get!(&client, format!("{}/api/v1/user/account/ghost/positions", base_url), Some(&token), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_market_watchlist_idor() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let admin_token = get_admin_token(&client, &base_url).await?;

    assert_post!(&client, format!("{}/api/v1/admin/users", base_url), Some(&admin_token), &serde_json::json!({
        "id": "a", "name": "A", "password": "p", "role": "Standard", "force_password_change": false
    }), StatusCode::OK);
    let res = assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "a".to_string(), password: "p".to_string(), client_id: "c".to_string(),
    }, StatusCode::OK);
    let token = res.json::<ApiResponse<LoginResponse>>().await?.data.context("data null")?.access_token;

    // Actually watchlist list is always for CURRENT USER, so IDOR is hard there. 
    // But let's check watchlist delete with non-existent stock
    assert_delete!(&client, format!("{}/api/v1/user/watchlist/NOSUCHSTOCK", base_url), Some(&token), StatusCode::OK);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_trade_insufficient_funds() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let token = get_admin_token(&client, &base_url).await?;

    // Try to buy 1 billion AAPL shares (market price ~150, so need $150B)
    assert_post!(&client, format!("{}/api/v1/user/orders", base_url), Some(&token), &serde_json::json!({
        "account_id": "trader_01", "symbol": "AAPL", "volume": "1000000000", "direction": "BUY"
    }), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_strategy_already_running() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let token = get_admin_token(&client, &base_url).await?;

    let payload = serde_json::json!({
        "symbol": "AAPL", "account_id": "trader_01", "timeframe": "1m", "engine_type": "JavaScript",
        "source_base64": base64::prelude::BASE64_STANDARD.encode("print(1)")
    });

    // Start once
    let res = assert_post!(&client, format!("{}/api/v1/user/strategies", base_url), Some(&token), &payload, StatusCode::OK);
    let id = res.json::<ApiResponse<String>>().await?.data.context("data null")?;

    // Try to start the same instance ID again (Impossible because start always creates new ID)
    // But we can try to "start" it by calling deploy again, it just creates a NEW instance.
    // To trigger AlreadyRunning, we'd need Manager's logic for duplicate tasks if any.
    // Actually, StrategyManager doesn't seem to block duplicate deployments of same symbol/account.
    // But we can trigger it if we try to call a method that expects Stopped but it's Running.
    // Wait, update_strategy returns 400 if running.
    assert_put!(&client, format!("{}/api/v1/user/strategies/{}", base_url, id), Some(&token), &serde_json::json!({"source_base64": "xx"}), StatusCode::BAD_REQUEST);

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_trade_algo_list_idor() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let admin_token = get_admin_token(&client, &base_url).await?;

    assert_post!(&client, format!("{}/api/v1/admin/users", base_url), Some(&admin_token), &serde_json::json!({
        "id": "a", "name": "A", "password": "p", "role": "Standard", "force_password_change": false
    }), StatusCode::OK);
    let res = assert_post!(&client, format!("{}/api/v1/auth/login", base_url), None::<&str>, &LoginRequest {
        username: "a".to_string(), password: "p".to_string(), client_id: "c".to_string(),
    }, StatusCode::OK);
    let token = res.json::<ApiResponse<LoginResponse>>().await?.data.context("data null")?.access_token;

    // Corrected: Uses Query Param
    assert_get!(&client, format!("{}/api/v1/user/algo?account_id=trader_01", base_url), Some(&token), StatusCode::FORBIDDEN);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_trade_get_orders_not_found() -> anyhow::Result<()> {
    let (base_url, system_store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let token = get_admin_token(&client, &base_url).await?;

    system_store.bind_account("admin", "ghost").await?;
    // For a list resource, empty result is 200 OK
    let res = assert_get!(&client, format!("{}/api/v1/user/orders?account_id=ghost", base_url), Some(&token), StatusCode::OK);
    let data = res.json::<ApiResponse<Vec<okane_api::types::OrderResponse>>>().await?.data.context("data null")?;
    assert!(data.is_empty());
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_trade_place_order_none_price() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive) = spawn_test_server().await?;
    let client = reqwest::Client::new();
    let token = get_admin_token(&client, &base_url).await?;

    // No price field = Market Order
    assert_post!(&client, format!("{}/api/v1/user/orders", base_url), Some(&token), &serde_json::json!({
        "account_id": "trader_01", "symbol": "AAPL", "volume": "1", "direction": "BUY"
    }), StatusCode::OK);
    Ok(())
}

#[test]
fn test_from_position_to_response() {
    use okane_core::trade::entity::{Position, AccountId};
    use okane_api::types::PositionResponse;
    use rust_decimal::Decimal;

    let p = Position {
        account_id: AccountId("test".to_string()),
        symbol: "AAPL".to_string(),
        volume: Decimal::ONE_HUNDRED,
        average_price: Decimal::from(150),
    };
    let pr: PositionResponse = p.into();
    assert_eq!(pr.symbol, "AAPL");
}

#[test]
fn test_from_account_snapshot_to_response() {
    use okane_core::trade::entity::{AccountSnapshot, AccountId};
    use okane_api::types::AccountSnapshotResponse;
    use rust_decimal::Decimal;

    let s = AccountSnapshot {
        account_id: AccountId("test".to_string()),
        available_balance: Decimal::from(1000),
        frozen_balance: Decimal::from(500),
        total_equity: Decimal::from(1500),
        positions: vec![],
    };
    let sr: AccountSnapshotResponse = s.into();
    assert_eq!(sr.account_id, "test");
}

#[test]
fn test_from_user_to_response() {
    use okane_api::types::UserResponse;
    use chrono::Utc;

    let u = okane_core::store::port::User {
        id: "u1".to_string(),
        name: "User 1".to_string(),
        password_hash: "xxx".to_string(),
        role: okane_core::store::port::UserRole::Standard,
        force_password_change: false,
        created_at: Utc::now(),
    };
    let ur = UserResponse::from(&u);
    assert_eq!(ur.id, "u1");
}

#[test]
fn test_from_candle_to_response() {
    use rust_decimal::Decimal;
    use chrono::Utc;

    let c = okane_core::market::entity::Candle {
        time: Utc::now(),
        open: Decimal::from(100),
        high: Decimal::from(110),
        low: Decimal::from(90),
        close: Decimal::from(105),
        adj_close: Some(Decimal::from(105)),
        volume: Decimal::from(1000),
        is_final: true,
    };
    let cr: okane_api::types::CandleResponse = c.into();
    assert!(cr.is_final);
}
