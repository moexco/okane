pub mod mock_trade;
use async_trait::async_trait;
use chrono::Utc;
use okane_core::common::time::FakeClockProvider;
use okane_core::common::{Stock as StockIdentity, TimeFrame};
use okane_core::engine::entity::{Signal, SignalKind};
use okane_core::engine::error::EngineError;
use okane_core::engine::port::SignalHandler;
use okane_core::market::entity::Candle;
use okane_core::market::error::MarketError;
use okane_core::market::port::{CandleStream, Market, Stock, StockStatus};
use okane_engine::quickjs::JsEngine;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

struct MockStock {
    identity: StockIdentity,
    price_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<Candle>>>,
}

#[async_trait]
impl Stock for MockStock {
    fn identity(&self) -> &StockIdentity {
        &self.identity
    }
    fn current_price(&self) -> Option<f64> {
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
        _: usize,
        _: Option<chrono::DateTime<Utc>>,
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
}

struct MockHandler {
    captured: Arc<Mutex<Vec<Signal>>>,
}

#[async_trait]
impl SignalHandler for MockHandler {
    fn matches(&self, _: &Signal) -> bool {
        true
    }
    async fn handle(&self, signal: Signal) -> Result<(), EngineError> {
        self.captured.lock().unwrap().push(signal);
        Ok(())
    }
}

/// JS 策略源码：close > 150 时产生 LongEntry 信号
const JS_STRATEGY: &str = r#"
function onCandle(input) {
    var candle = JSON.parse(input);
    if (candle.close > 150.0) {
        return JSON.stringify({
            id: "sig_js_001",
            symbol: "AAPL",
            timestamp: new Date().toISOString(),
            kind: "LongEntry",
            strategy_id: "js-test-strategy",
            metadata: {}
        });
    }
    return "null";
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
    let mut engine = JsEngine::new(market, trade, Arc::new(FakeClockProvider::new(Utc::now())));
    let captured_signals = Arc::new(Mutex::new(Vec::new()));
    engine.register_handler(Box::new(MockHandler {
        captured: captured_signals.clone(),
    }));

    // QuickJS 不是 Send，使用 LocalSet + spawn_local
    let local = tokio::task::LocalSet::new();

    let handle = local.spawn_local(async move {
        engine
            .run_strategy("AAPL", "mock_account", TimeFrame::Minute1, JS_STRATEGY)
            .await
    });

    local
        .run_until(async {
            // 推送触发信号的数据 (close > 150.0)
            tx.send(Candle {
                time: Utc::now(),
                open: 100.0,
                high: 160.0,
                low: 90.0,
                close: 155.0,
                adj_close: None,
                volume: 1000.0,
                is_final: true,
            })
            .unwrap();

            // 给处理时间
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            // 验证结果
            let signals = captured_signals.lock().unwrap();
            assert_eq!(signals.len(), 1, "Should have captured 1 signal");
            assert_eq!(signals[0].kind, SignalKind::LongEntry);
            assert_eq!(signals[0].strategy_id, "js-test-strategy");

            handle.abort();
        })
        .await;
}

#[tokio::test]
async fn test_js_strategy_no_signal_when_below_threshold() {
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
    let mut engine = JsEngine::new(market, trade, Arc::new(FakeClockProvider::new(Utc::now())));
    let captured_signals = Arc::new(Mutex::new(Vec::new()));
    engine.register_handler(Box::new(MockHandler {
        captured: captured_signals.clone(),
    }));

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
                open: 100.0,
                high: 105.0,
                low: 95.0,
                close: 100.0,
                adj_close: None,
                volume: 500.0,
                is_final: true,
            })
            .unwrap();

            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            let signals = captured_signals.lock().unwrap();
            assert_eq!(signals.len(), 0, "Should NOT have captured any signal");

            handle.abort();
        })
        .await;
}
