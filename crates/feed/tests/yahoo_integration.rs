use chrono::{Duration as ChronoDuration, Utc};
use futures::StreamExt;
use okane_core::common::{Stock, TimeFrame};
use okane_core::market::port::MarketDataProvider;
use okane_feed::yahoo::YahooProvider;
use std::time::Duration;
use tokio::time::timeout;

/// # Summary
/// 雅虎财经行情获取的集成测试。
///
/// # Logic
/// 1. 初始化 YahooProvider。
/// 2. 抓取 AAPL 过去 7 天的日线数据。
/// 3. 断言数据非空且返回成功。
#[tokio::test]
async fn test_yahoo_real_fetch() {
    let provider = YahooProvider::new();
    let stock = Stock {
        symbol: "AAPL".to_string(),
        exchange: None,
    };
    let end = Utc::now();
    let start = end - ChronoDuration::days(7);

    let result = provider
        .fetch_candles(&stock, TimeFrame::Day1, start, end)
        .await;

    assert!(
        result.is_ok(),
        "Failed to fetch real data from Yahoo: {:?}",
        result.err()
    );
    let candles = result.unwrap();
    assert!(!candles.is_empty(), "Candles list should not be empty");

    println!("Successfully fetched {} candles for AAPL", candles.len());
    for candle in candles.iter() {
        println!("{:?}: Close = {}", candle.time, candle.close);
    }
}

/// # Summary
/// 雅虎财经流式订阅的集成测试。
///
/// # Logic
/// 1. 初始化 YahooProvider 并订阅 AAPL 的日线流。
/// 2. 验证流是否能产生至少一个数据点（初始订阅会触发一次即时抓取）。
/// 3. 设置 30 秒超时以应对网络波动。
#[tokio::test]
async fn test_yahoo_stream_subscribe() {
    let provider = YahooProvider::new();
    let stock = Stock {
        symbol: "AAPL".to_string(),
        exchange: None,
    };

    println!("正在订阅 {} 的流式数据...", stock.symbol);
    let result = provider.subscribe_candles(&stock, TimeFrame::Day1).await;

    assert!(result.is_ok(), "订阅流失败: {:?}", result.err());
    let mut stream = result.unwrap();

    // 初始订阅后，内部的第一个 tick 会立即执行一次获取
    println!("等待第一条推送数据...");
    let first_item = timeout(Duration::from_secs(30), stream.next()).await;

    assert!(first_item.is_ok(), "流数据推送超时（30s）");
    let candle_opt = first_item.unwrap();
    assert!(candle_opt.is_some(), "流已关闭且未收到数据");

    let candle = candle_opt.unwrap();
    println!(
        "收到流式数据 -> 时间: {:?}, 收盘价: {}",
        candle.time, candle.close
    );
    assert!(candle.close > 0.0);
}

/// # Summary
/// 雅虎财经证券搜索的集成测试。
///
/// # Logic
/// 1. 初始化 YahooProvider。
/// 2. 搜索关键词 "Apple"。
/// 3. 断言返回结果中包含 "AAPL" 股票。
#[tokio::test]
async fn test_yahoo_search_symbols() {
    let provider = YahooProvider::new();
    let query = "Apple";

    println!("正在搜索关键词: {}...", query);
    let result = provider.search_symbols(query).await;

    assert!(result.is_ok(), "搜索符号失败: {:?}", result.err());
    let symbols = result.unwrap();

    assert!(!symbols.is_empty(), "搜索结果不应为空");
    
    println!("找到 {} 个匹配项:", symbols.len());
    for s in &symbols {
        println!("- {} ({}): {} {}", s.symbol, s.exchange, s.name, s.currency);
    }

    // 检查是否包含 AAPL
    let has_aapl = symbols.iter().any(|s| s.symbol == "AAPL");
    assert!(has_aapl, "搜索结果应包含 AAPL");
}
