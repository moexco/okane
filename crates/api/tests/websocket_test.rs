mod common;

use chrono::Utc;
use common::spawn_test_server;
use futures::StreamExt;
use okane_api::types::{ApiResponse, LoginRequest, LoginResponse};
use okane_core::market::entity::Candle;
use reqwest::StatusCode;
use rust_decimal_macros::dec;
use tokio_tungstenite::connect_async;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_market_websocket_push() -> anyhow::Result<()> {
    let (base_url, _store, _tmp, _keepalive, feed) = spawn_test_server().await?;
    let client = reqwest::Client::new();

    // 1. 登录获取 Token
    let res = assert_post!(
        &client,
        format!("{}/api/v1/auth/login", base_url),
        None::<&str>,
        &LoginRequest {
            username: "admin".to_string(),
            password: "test_admin_pwd".to_string(),
            client_id: "ws_test_client".to_string(),
        },
        StatusCode::OK
    );

    let login_data: ApiResponse<LoginResponse> = res.json().await?;
    let token = login_data
        .data
        .ok_or_else(|| anyhow::anyhow!("login response missing tokens"))?
        .access_token;

    // 2. 建立 WebSocket 连接
    // 将 http:// 转为 ws://
    let ws_url = base_url.replace("http://", "ws://");
    let ws_endpoint = format!("{}/api/v1/market/ws/AAPL?tf=1m", ws_url);
    let host = base_url.trim_start_matches("http://");
    // ("Connecting to WS: {}", ws_endpoint);

    let request = http::Request::builder()
        .uri(&ws_endpoint)
        .header("Host", host)
        .header("Authorization", format!("Bearer {}", token))
        .header(
            "Sec-WebSocket-Key",
            tokio_tungstenite::tungstenite::handshake::client::generate_key(),
        )
        .header("Sec-WebSocket-Version", "13")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .body(())?;

    let (mut ws_stream, _) = connect_async(request)
        .await
        .map_err(|e| anyhow::anyhow!("WS Connect error: {}", e))?;

    // 3. 模拟推送行情
    let test_candle = Candle {
        time: Utc::now(),
        open: dec!(150.0),
        high: dec!(151.0),
        low: dec!(149.0),
        close: dec!(150.5),
        adj_close: None,
        volume: dec!(1000.0),
        is_final: false,
    };

    // 稍微等待确保 WS 订阅已完成
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    feed.push_candle(test_candle.clone());

    // 4. 验证接收到的数据
    let msg = ws_stream
        .next()
        .await
        .ok_or_else(|| anyhow::anyhow!("WS stream closed without receiving data"))?;
    let msg = msg.map_err(|e| anyhow::anyhow!("WS Recv error: {}", e))?;

    if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
        let received: Candle = serde_json::from_str(&text)?;
        assert_eq!(received.close, test_candle.close);
        assert_eq!(received.open, test_candle.open);
    } else {
        anyhow::bail!("Expected text message, got {:?}", msg);
    }

    Ok(())
}
