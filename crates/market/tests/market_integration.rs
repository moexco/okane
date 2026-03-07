use okane_core::test_utils::{MockMarketDataProvider, MemMarketStore, wait_for_condition};
use std::sync::Arc;
use okane_core::market::port::{Market, StockStatus};
use okane_market::manager::MarketImpl;
use okane_core::market::entity::Candle;
use chrono::Utc;
use rust_decimal_macros::dec;
use okane_core::common::TimeFrame;
use futures::StreamExt;
use tokio::time::{Duration, sleep};

/// # Summary
/// 初始化测试所需的 Market 环境。
async fn setup() -> (Arc<MarketImpl>, Arc<MockMarketDataProvider>) {
    let provider = Arc::new(MockMarketDataProvider::new());
    let store = Arc::new(MemMarketStore::new());
    let market = MarketImpl::new(provider.clone(), store);
    (market, provider)
}

#[tokio::test]
async fn test_get_stock_instance() -> anyhow::Result<()> {
    let (market, _) = setup().await;
    let stock = market.get_stock("AAPL").await.map_err(|e| anyhow::anyhow!(e))?;
    assert_eq!(stock.identity().symbol, "AAPL");
    Ok(())
}

#[tokio::test]
async fn test_stock_identity() -> anyhow::Result<()> {
    let (market, _) = setup().await;
    let stock = market.get_stock("AAPL").await.map_err(|e| anyhow::anyhow!(e))?;
    let id = stock.identity();
    assert_eq!(id.symbol, "AAPL");
    Ok(())
}

#[tokio::test]
async fn test_stock_current_price() -> anyhow::Result<()> {
    let (market, provider) = setup().await;
    let stock = market.get_stock("AAPL").await.map_err(|e| anyhow::anyhow!(e))?;

    // 初始状态无价格
    assert!(stock.current_price().map_err(|e| anyhow::anyhow!(e))?.is_none());

    // 模拟推送价格
    provider.push_candle(Candle {
        time: Utc::now(),
        open: dec!(100.0),
        high: dec!(100.0),
        low: dec!(100.0),
        close: dec!(155.5),
        adj_close: None,
        volume: dec!(1.0),
        is_final: false,
    });

    // 等待 Fetcher 处理
    let success = wait_for_condition(Duration::from_secs(1), Duration::from_millis(10), || async {
        stock.current_price()
            .map(|p| p == Some(dec!(155.5)))
            .unwrap_or_else(|e| {
                tracing::error!("Test error during current_price poll: {}", e);
                false
            })
    }).await;
    assert!(success, "Should update price within 1s");
    Ok(())
}

#[tokio::test]
async fn test_stock_latest_candle() -> anyhow::Result<()> {
    let (market, provider) = setup().await;
    let stock = market.get_stock("AAPL").await.map_err(|e| anyhow::anyhow!(e))?;

    provider.push_candle(Candle {
        time: Utc::now(),
        open: dec!(100.0),
        high: dec!(110.0),
        low: dec!(90.0),
        close: dec!(105.0),
        adj_close: None,
        volume: dec!(100.0),
        is_final: false,
    });

    let success = wait_for_condition(Duration::from_secs(1), Duration::from_millis(10), || async {
        stock.latest_candle(TimeFrame::Minute1)
            .map(|c| c.is_some())
            .unwrap_or_else(|e| {
                tracing::error!("Test error during latest_candle poll: {}", e);
                false
            })
    }).await;
    assert!(success, "Should receive latest candle within 1s");
    assert_eq!(stock.latest_candle(TimeFrame::Minute1).map_err(|e| anyhow::anyhow!(e))?.ok_or_else(|| anyhow::anyhow!("Candle null"))?.close, dec!(105.0));
    Ok(())
}

#[tokio::test]
async fn test_stock_last_closed_candle() -> anyhow::Result<()> {
    let (market, provider) = setup().await;
    let stock = market.get_stock("AAPL").await.map_err(|e| anyhow::anyhow!(e))?;

    // 推送未闭合的
    provider.push_candle(Candle {
        time: Utc::now(),
        open: dec!(100.0),
        high: dec!(110.0),
        low: dec!(90.0),
        close: dec!(105.0),
        adj_close: None,
        volume: dec!(100.0),
        is_final: false,
    });
    // 确保处理完成但状态不对
    sleep(Duration::from_millis(20)).await; 
    assert!(stock.last_closed_candle(TimeFrame::Minute1).map_err(|e| anyhow::anyhow!(e))?.is_none());

    // 推送已闭合的
    provider.push_candle(Candle {
        time: Utc::now(),
        open: dec!(100.0),
        high: dec!(110.0),
        low: dec!(90.0),
        close: dec!(108.0),
        adj_close: None,
        volume: dec!(100.0),
        is_final: true,
    });
    let success = wait_for_condition(Duration::from_secs(1), Duration::from_millis(10), || async {
        stock.last_closed_candle(TimeFrame::Minute1)
            .map(|c| c.is_some())
            .unwrap_or_else(|e| {
                tracing::error!("Test error during last_closed_candle poll: {}", e);
                false
            })
    }).await;
    assert!(success, "Should receive closed candle within 1s");
    assert_eq!(stock.last_closed_candle(TimeFrame::Minute1).map_err(|e| anyhow::anyhow!(e))?.ok_or_else(|| anyhow::anyhow!("Closed candle null"))?.close, dec!(108.0));
    Ok(())
}

#[tokio::test]
async fn test_stock_subscribe() -> anyhow::Result<()> {
    let (market, provider) = setup().await;
    let stock = market.get_stock("AAPL").await.map_err(|e| anyhow::anyhow!(e))?;
    let mut stream = stock.subscribe(TimeFrame::Minute1).map_err(|e| anyhow::anyhow!(e))?;

    provider.push_candle(Candle {
        time: Utc::now(),
        open: dec!(1.0),
        high: dec!(1.0),
        low: dec!(1.0),
        close: dec!(99.0),
        adj_close: None,
        volume: dec!(1.0),
        is_final: true,
    });

    let received = stream.next().await;
    assert!(received.is_some());
    assert_eq!(received.ok_or_else(|| anyhow::anyhow!("Received null"))?.close, dec!(99.0));
    Ok(())
}

#[tokio::test]
async fn test_stock_fetch_history() -> anyhow::Result<()> {
    let (market, provider) = setup().await;
    let stock = market.get_stock("AAPL").await.map_err(|e| anyhow::anyhow!(e))?;
    
    // 模拟种子数据
    let now = Utc::now();
    provider.set_history(vec![Candle {
        time: now - chrono::Duration::hours(1),
        open: dec!(100.0),
        high: dec!(105.0),
        low: dec!(95.0),
        close: dec!(102.0),
        adj_close: None,
        volume: dec!(1000.0),
        is_final: true,
    }]).map_err(|e| anyhow::anyhow!(e))?;

    let end = Utc::now();
    let start = end - chrono::Duration::days(10);
    let history = stock.fetch_history(TimeFrame::Day1, start, end).await.map_err(|e| anyhow::anyhow!(e))?;
    assert!(!history.is_empty());
    assert_eq!(history[0].close, dec!(102.0));
    Ok(())
}

#[tokio::test]
async fn test_stock_status() -> anyhow::Result<()> {
    let (market, _) = setup().await;
    let stock = market.get_stock("AAPL").await.map_err(|e| anyhow::anyhow!(e))?;
    assert_eq!(stock.status(), StockStatus::Online);
    Ok(())
}
