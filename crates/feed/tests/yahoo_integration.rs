use chrono::{Duration as ChronoDuration, Utc};
use futures::StreamExt;
use okane_core::common::{Stock, TimeFrame};
use okane_core::market::port::MarketDataProvider;
use okane_feed::yahoo::YahooProvider;
use std::time::Duration;
use tokio::time::timeout;

/// # Summary
/// 初始化 Rustls 加密提供程序。
fn init_crypto() {
    okane_core::common::install_rustls_crypto_provider();
}

/// # Summary
/// 雅虎财经行情获取的集成测试。
///
/// # Logic
/// 1. 初始化 Rustls 加密提供程序。
/// 2. 初始化 YahooProvider。
/// 3. 抓取 AAPL 过去 7 天的日线数据。
/// 4. 断言数据非空且返回成功。
#[tokio::test]
#[ignore]
async fn test_yahoo_real_fetch() -> Result<(), Box<dyn std::error::Error>> {
    init_crypto();
    let provider = YahooProvider::new()?;
    let stock = Stock {
        symbol: "AAPL".to_string(),
        exchange: None,
    };
    let end = Utc::now();
    let start = end - ChronoDuration::days(7);

    let candles = provider
        .fetch_candles(&stock, TimeFrame::Day1, start, end)
        .await?;

    assert!(!candles.is_empty(), "Candles list should not be empty");
    assert!(
        candles
            .iter()
            .all(|c| c.close > rust_decimal::Decimal::ZERO),
        "All candles should have positive close price"
    );

    Ok(())
}

/// # Summary
/// 雅虎财经流式订阅的集成测试。
///
/// # Logic
/// 1. 初始化 YahooProvider 并订阅 AAPL 的日线流。
/// 2. 验证流是否能产生至少一个数据点（Yahoo 首条 tick 可能只有价格，没有 volume delta）。
/// 3. 设置 30 秒超时以应对网络波动。
#[tokio::test]
#[ignore]
async fn test_yahoo_stream_subscribe() -> Result<(), Box<dyn std::error::Error>> {
    init_crypto();
    let provider = YahooProvider::new()?;
    let stock = Stock {
        symbol: "AAPL".to_string(),
        exchange: None,
    };

    let mut stream = provider.subscribe_candles(&stock).await?;

    // 初始订阅后，WS 或 fallback polling 连接并等待数据
    let first_item = timeout(Duration::from_secs(90), stream.next()).await?;
    let candle = first_item.ok_or("流已关闭且未收到数据")??;

    assert!(candle.close > rust_decimal::Decimal::ZERO);
    assert!(!candle.is_final);
    assert!(candle.volume >= rust_decimal::Decimal::ZERO);
    Ok(())
}

/// # Summary
/// 雅虎财经证券搜索的集成测试。
///
/// # Logic
/// 1. 初始化 YahooProvider。
/// 2. 搜索关键词 "Apple"。
/// 3. 断言返回结果中包含 "AAPL" 股票。
#[tokio::test]
#[ignore]
async fn test_yahoo_search_symbols() -> Result<(), Box<dyn std::error::Error>> {
    init_crypto();
    let provider = YahooProvider::new()?;
    let query = "Apple";

    let symbols = provider.search_symbols(query).await?;

    assert!(!symbols.is_empty(), "搜索结果不应为空");

    // 检查是否包含 AAPL
    let has_aapl = symbols.iter().any(|s| s.symbol == "AAPL");
    assert!(has_aapl, "搜索结果应包含 AAPL");
    Ok(())
}
