pub mod mock_trade;
use async_trait::async_trait;
use chrono::Utc;
use okane_core::common::time::FakeClockProvider;
use okane_core::common::{Stock as StockIdentity, TimeFrame};
use okane_core::market::entity::Candle;
use okane_core::market::error::MarketError;
use okane_core::market::port::{CandleStream, Market, Stock, StockStatus};
use okane_engine::quickjs::JsEngine;
use std::sync::Arc;
use tokio::sync::mpsc;
use rust_decimal_macros::dec;

struct MockStock {
    identity: StockIdentity,
    price_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<Candle>>>,
}

#[async_trait]
impl Stock for MockStock {
    fn identity(&self) -> &StockIdentity {
        &self.identity
    }
    fn current_price(&self) -> Option<rust_decimal::Decimal> {
        None
    }
    fn latest_candle(&self, _: TimeFrame) -> Option<Candle> {
        None
    }
    fn last_closed_candle(&self, _: TimeFrame) -> Option<Candle> {
        None
    }
    fn status(&self) -> StockStatus {
        StockStatus::Online
    }

    fn subscribe(&self, _: TimeFrame) -> CandleStream {
        let rx = self.price_rx.clone();
        let s = async_stream::stream! {
            let mut rx = rx.lock().await;
            while let Some(candle) = rx.recv().await {
                yield candle;
            }
        };
        Box::pin(s)
    }

    async fn fetch_history(
        &self,
        _: TimeFrame,
        _: chrono::DateTime<Utc>,
        _: chrono::DateTime<Utc>,
    ) -> Result<Vec<Candle>, MarketError> {
        Ok(vec![])
    }
}

struct MockMarket {
    stock: Arc<MockStock>,
}

#[async_trait]
impl Market for MockMarket {
    async fn get_stock(&self, _: &str) -> Result<Arc<dyn Stock>, MarketError> {
        Ok(self.stock.clone())
    }

    async fn search_symbols(&self, _query: &str) -> Result<Vec<okane_core::store::port::StockMetadata>, MarketError> {
        Ok(vec![])
    }
}

/// JS 策略源码：验证 onCandle 调用正常（void 返回）
const JS_STRATEGY: &str = r#"
function onCandle(input) {
    var candle = JSON.parse(input);
    if (candle.close > 150.0) {
        host.log(0, "Breakout detected: close=" + candle.close);
    }
}
"#;

#[tokio::test]
async fn test_js_strategy_integration() {
    let (tx, rx) = mpsc::unbounded_channel();
    let mock_stock = Arc::new(MockStock {
        identity: StockIdentity {
            symbol: "AAPL".to_string(),
            exchange: Some("NASDAQ".to_string()),
        },
        price_rx: Arc::new(tokio::sync::Mutex::new(rx)),
    });
    let market = Arc::new(MockMarket {
        stock: mock_stock.clone(),
    });

    let trade = std::sync::Arc::new(crate::mock_trade::MockTradePort);
    let engine = JsEngine::new(market, trade, Arc::new(FakeClockProvider::new(Utc::now())), None);

    let local = tokio::task::LocalSet::new();

    let handle = local.spawn_local(async move {
        engine
            .run_strategy("AAPL", "mock_account", TimeFrame::Minute1, JS_STRATEGY)
            .await
    });

    local
        .run_until(async {
            // 推送 K 线数据, close > 150 触发 host.log
            tx.send(Candle {
                time: Utc::now(),
                open: dec!(100.0),
                high: dec!(160.0),
                low: dec!(90.0),
                close: dec!(155.0),
                adj_close: None,
                volume: dec!(1000.0),
                is_final: true,
            })
            .unwrap();

            // 给处理时间
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            // 策略应正常执行 (void onCandle)
            handle.abort();
        })
        .await;
}

#[tokio::test]
async fn test_js_strategy_no_error_when_below_threshold() {
    let (tx, rx) = mpsc::unbounded_channel();
    let mock_stock = Arc::new(MockStock {
        identity: StockIdentity {
            symbol: "AAPL".to_string(),
            exchange: None,
        },
        price_rx: Arc::new(tokio::sync::Mutex::new(rx)),
    });
    let market = Arc::new(MockMarket {
        stock: mock_stock.clone(),
    });

    let trade = std::sync::Arc::new(crate::mock_trade::MockTradePort);
    let engine = JsEngine::new(market, trade, Arc::new(FakeClockProvider::new(Utc::now())), None);

    let local = tokio::task::LocalSet::new();

    let handle = local.spawn_local(async move {
        engine
            .run_strategy("AAPL", "mock_account", TimeFrame::Minute1, JS_STRATEGY)
            .await
    });

    local
        .run_until(async {
            tx.send(Candle {
                time: Utc::now(),
                open: dec!(100.0),
                high: dec!(105.0),
                low: dec!(95.0),
                close: dec!(100.0),
                adj_close: None,
                volume: dec!(500.0),
                is_final: true,
            })
            .unwrap();

            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            // 策略应正常执行 — 不触发 log 但也不报错
            handle.abort();
        })
        .await;
}
