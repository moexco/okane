pub mod mock_trade;
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use okane_core::common::time::FakeClockProvider;
use okane_core::common::{Stock as StockIdentity, TimeFrame};
use okane_core::engine::entity::Signal;
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
    rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<Candle>>>,
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
        let rx = self.rx.clone();
        let s = async_stream::stream! {
            let mut rx = rx.lock().await;
            while let Some(c) = rx.recv().await { yield c; }
        };
        Box::pin(s)
    }
    async fn fetch_history(
        &self,
        _: TimeFrame,
        _start: chrono::DateTime<Utc>,
        end: chrono::DateTime<Utc>,
    ) -> Result<Vec<Candle>, MarketError> {
        let mut candles = Vec::new();
        let base_time = end;
        for i in 0..5 { // 测试逻辑中期望是 5 根
            candles.push(Candle {
                time: base_time - chrono::Duration::minutes(i as i64),
                open: 100.0,
                high: 100.0,
                low: 100.0,
                close: 100.0,
                adj_close: None,
                volume: 0.0,
                is_final: true,
            });
        }
        Ok(candles)
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

/// JS 策略：验证 host.now() 和 host.fetchHistory() 调用
const JS_HOST_TEST_STRATEGY: &str = r#"
function onCandle(input) {
    var nowMs = host.now();
    host.log(3, "JS logic time check: " + nowMs);

    var historyJson = host.fetchHistory("AAPL", "1m", 5);
    var history = JSON.parse(historyJson);

    return JSON.stringify({
        id: "host-test-001",
        symbol: "AAPL",
        timestamp: new Date().toISOString(),
        kind: "Info",
        strategy_id: "host-test",
        metadata: {
            history_count: String(history.length),
            logical_now: String(nowMs)
        }
    });
}
"#;

#[tokio::test]
async fn test_host_functions_from_js() {
    let (tx, rx) = mpsc::unbounded_channel();
    let mock_stock = Arc::new(MockStock {
        identity: StockIdentity {
            symbol: "AAPL".to_string(),
            exchange: None,
        },
        rx: Arc::new(tokio::sync::Mutex::new(rx)),
    });
    let market = Arc::new(MockMarket { stock: mock_stock });
    let trade = std::sync::Arc::new(crate::mock_trade::MockTradePort);
    let time_provider = Arc::new(FakeClockProvider::new(Utc::now()));
    let time_provider_clone = time_provider.clone();
    let mut engine = JsEngine::new(market, trade, time_provider);
    let captured = Arc::new(Mutex::new(Vec::new()));
    engine.register_handler(Box::new(MockHandler {
        captured: captured.clone(),
    }));

    let local = tokio::task::LocalSet::new();

    let handle = local.spawn_local(async move {
        engine
            .run_strategy("AAPL", "mock_account", TimeFrame::Minute1, JS_HOST_TEST_STRATEGY)
            .await
    });

    local
        .run_until(async {
            let test_time = Utc.with_ymd_and_hms(2026, 2, 2, 10, 0, 0).unwrap();
            time_provider_clone.set_time(test_time);
            tx.send(Candle {
                time: test_time,
                open: 150.0,
                high: 150.0,
                low: 150.0,
                close: 150.0,
                adj_close: None,
                volume: 0.0,
                is_final: true,
            })
            .unwrap();

            tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

            let signals = captured.lock().unwrap();
            assert_eq!(signals.len(), 1, "Should have captured 1 signal");
            let sig = &signals[0];

            let logical_now = sig.metadata.get("logical_now").unwrap();
            let expected_ms = test_time.timestamp_millis().to_string();
            assert_eq!(
                logical_now, &expected_ms,
                "Logical time should match the candle time"
            );

            assert_eq!(
                sig.metadata.get("history_count").unwrap(),
                "5",
                "Should have fetched 5 historical candles"
            );

            handle.abort();
        })
        .await;
}
