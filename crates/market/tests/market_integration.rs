use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use futures::StreamExt;
use okane_core::common::{Stock as StockIdentity, TimeFrame};
use okane_core::market::entity::Candle;
use okane_core::market::error::MarketError;
use okane_core::market::port::{CandleStream, Market, MarketDataProvider, StockStatus};
use okane_core::store::error::StoreError;
use okane_core::store::port::MarketStore;
use okane_market::manager::MarketImpl;
use std::sync::Arc;
use tokio::sync::mpsc;

/// # Summary
/// 为测试提供的模拟行情驱动。
struct MockProvider {
    // 预设的价格数据，用于流推送
    price_tx: mpsc::UnboundedSender<Candle>,
    // 用于内部消费流
    price_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<Candle>>>,
}

impl MockProvider {
    fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            price_tx: tx,
            price_rx: Arc::new(tokio::sync::Mutex::new(rx)),
        }
    }

    fn push_candle(&self, candle: Candle) {
        let _ = self.price_tx.send(candle);
    }
}

#[async_trait]
impl MarketDataProvider for MockProvider {
    async fn fetch_candles(
        &self,
        _: &StockIdentity,
        _: TimeFrame,
        _: DateTime<Utc>,
        _: DateTime<Utc>,
    ) -> Result<Vec<Candle>, MarketError> {
        Ok(vec![Candle {
            time: Utc::now(),
            open: 150.0,
            high: 155.0,
            low: 149.0,
            close: 152.0,
            adj_close: None,
            volume: 1000.0,
            is_final: true,
        }])
    }

    async fn subscribe_candles(
        &self,
        _: &StockIdentity,
        _: TimeFrame,
    ) -> Result<CandleStream, MarketError> {
        let rx = self.price_rx.clone();
        let s = async_stream::stream! {
            let mut rx = rx.lock().await;
            while let Some(candle) = rx.recv().await {
                yield candle;
            }
        };
        Ok(Box::pin(s))
    }
}

/// # Summary
/// 基于内存的简易存储实现。
struct MemMarketStore {
    db: DashMap<(String, TimeFrame), Vec<Candle>>,
}

impl MemMarketStore {
    fn new() -> Self {
        Self { db: DashMap::new() }
    }
}

#[async_trait]
impl MarketStore for MemMarketStore {
    async fn save_candles(
        &self,
        stock: &StockIdentity,
        timeframe: TimeFrame,
        candles: &[Candle],
    ) -> Result<(), StoreError> {
        let mut entry = self
            .db
            .entry((stock.symbol.clone(), timeframe))
            .or_default();
        entry.extend_from_slice(candles);
        Ok(())
    }

    async fn load_candles(
        &self,
        stock: &StockIdentity,
        timeframe: TimeFrame,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Candle>, StoreError> {
        let candles = self
            .db
            .get(&(stock.symbol.clone(), timeframe))
            .map(|v| v.clone())
            .unwrap_or_default();
        Ok(candles
            .into_iter()
            .filter(|c| c.time >= start && c.time <= end)
            .collect())
    }
}

/// # Summary
/// 初始化测试所需的 Market 环境。
async fn setup() -> (Arc<MarketImpl>, Arc<MockProvider>) {
    let provider = Arc::new(MockProvider::new());
    let store = Arc::new(MemMarketStore::new());
    let market = MarketImpl::new(provider.clone(), store);
    (market, provider)
}

#[tokio::test]
async fn test_get_stock_instance() {
    let (market, _) = setup().await;
    let result = market.get_stock("AAPL").await;
    assert!(
        result.is_ok(),
        "Should successfully get stock aggregate root"
    );
    assert_eq!(result.unwrap().identity().symbol, "AAPL");
}

#[tokio::test]
async fn test_stock_identity() {
    let (market, _) = setup().await;
    let stock = market.get_stock("AAPL").await.unwrap();
    let id = stock.identity();
    assert_eq!(id.symbol, "AAPL");
}

#[tokio::test]
async fn test_stock_current_price() {
    let (market, provider) = setup().await;
    let stock = market.get_stock("AAPL").await.unwrap();

    // 初始状态无价格
    assert!(stock.current_price().is_none());

    // 模拟推送价格
    provider.push_candle(Candle {
        time: Utc::now(),
        open: 100.0,
        high: 100.0,
        low: 100.0,
        close: 155.5,
        adj_close: None,
        volume: 1.0,
        is_final: false,
    });

    // 等待 Fetcher 处理
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    assert_eq!(stock.current_price(), Some(155.5));
}

#[tokio::test]
async fn test_stock_latest_candle() {
    let (market, provider) = setup().await;
    let stock = market.get_stock("AAPL").await.unwrap();

    provider.push_candle(Candle {
        time: Utc::now(),
        open: 100.0,
        high: 110.0,
        low: 90.0,
        close: 105.0,
        adj_close: None,
        volume: 100.0,
        is_final: false,
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    let candle = stock.latest_candle(TimeFrame::Minute1);
    assert!(candle.is_some());
    assert_eq!(candle.unwrap().close, 105.0);
}

#[tokio::test]
async fn test_stock_last_closed_candle() {
    let (market, provider) = setup().await;
    let stock = market.get_stock("AAPL").await.unwrap();

    // 推送未闭合的
    provider.push_candle(Candle {
        time: Utc::now(),
        open: 100.0,
        high: 110.0,
        low: 90.0,
        close: 105.0,
        adj_close: None,
        volume: 100.0,
        is_final: false,
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    assert!(stock.last_closed_candle(TimeFrame::Minute1).is_none());

    // 推送已闭合的
    provider.push_candle(Candle {
        time: Utc::now(),
        open: 100.0,
        high: 110.0,
        low: 90.0,
        close: 108.0,
        adj_close: None,
        volume: 100.0,
        is_final: true,
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    let closed = stock.last_closed_candle(TimeFrame::Minute1);
    assert!(closed.is_some());
    assert_eq!(closed.unwrap().close, 108.0);
}

#[tokio::test]
async fn test_stock_subscribe() {
    let (market, provider) = setup().await;
    let stock = market.get_stock("AAPL").await.unwrap();
    let mut stream = stock.subscribe(TimeFrame::Minute1);

    provider.push_candle(Candle {
        time: Utc::now(),
        open: 1.0,
        high: 1.0,
        low: 1.0,
        close: 99.0,
        adj_close: None,
        volume: 1.0,
        is_final: true,
    });

    let received = stream.next().await;
    assert!(received.is_some());
    assert_eq!(received.unwrap().close, 99.0);
}

#[tokio::test]
async fn test_stock_fetch_history() {
    let (market, _) = setup().await;
    let stock = market.get_stock("AAPL").await.unwrap();
    let history = stock.fetch_history(TimeFrame::Day1, 10).await;
    assert!(history.is_ok());
    assert!(!history.unwrap().is_empty());
}

#[tokio::test]
async fn test_stock_status() {
    let (market, _) = setup().await;
    let stock = market.get_stock("AAPL").await.unwrap();
    assert_eq!(stock.status(), StockStatus::Online);
}
