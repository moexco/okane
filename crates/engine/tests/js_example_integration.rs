pub mod mock_trade;
use async_trait::async_trait;
use chrono::Utc;
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
        limit: usize,
        end_at: Option<chrono::DateTime<Utc>>,
    ) -> Result<Vec<Candle>, MarketError> {
        // 返回足够数量的历史 K 线，close=100.0，使 SMA10=100.0
        let base_time = end_at.unwrap_or_else(Utc::now);
        let mut candles = Vec::new();
        for i in 0..limit {
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

/// 从文件系统加载 JS 示例策略
fn load_js_example() -> String {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let js_path = workspace_root.join("examples/js-strategy-demo/strategy.js");
    std::fs::read_to_string(&js_path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {}", js_path.display(), e))
}

/// 测试 JS 示例策略：满足条件时应发出 LongEntry 信号
/// 条件：close > SMA10 (100.0) && volume > 500 && is_final = true
#[tokio::test]
async fn test_js_example_strategy_signal() {
    let js_source = load_js_example();

    let (tx, rx) = mpsc::unbounded_channel();
    let mock_stock = Arc::new(MockStock {
        identity: StockIdentity {
            symbol: "AAPL".to_string(),
            exchange: None,
        },
        rx: Arc::new(tokio::sync::Mutex::new(rx)),
    });
    let market = Arc::new(MockMarket { stock: mock_stock });
    let trade = std::sync::Arc::new(crate::mock_trade::MockTradePort); let mut engine = JsEngine::new(market, trade);
    let captured = Arc::new(Mutex::new(Vec::new()));
    engine.register_handler(Box::new(MockHandler {
        captured: captured.clone(),
    }));

    let local = tokio::task::LocalSet::new();
    let handle = local.spawn_local(async move {
        engine
            .run_strategy("AAPL", "mock_account", TimeFrame::Minute1, &js_source)
            .await
    });

    local
        .run_until(async {
            // close=150.0 > SMA10(100.0), volume=1000 > 500, is_final=true → 触发
            tx.send(Candle {
                time: Utc::now(),
                open: 100.0,
                high: 155.0,
                low: 95.0,
                close: 150.0,
                adj_close: None,
                volume: 1000.0,
                is_final: true,
            })
            .unwrap();

            tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

            let signals = captured.lock().unwrap();
            assert_eq!(signals.len(), 1, "Should have captured 1 signal from JS example");
            assert_eq!(signals[0].kind, SignalKind::LongEntry);
            assert_eq!(signals[0].strategy_id, "js-ema-breakout");

            handle.abort();
        })
        .await;
}

/// 测试 JS 示例策略：is_final=false 时应跳过
#[tokio::test]
async fn test_js_example_strategy_skip_non_final() {
    let js_source = load_js_example();

    let (tx, rx) = mpsc::unbounded_channel();
    let mock_stock = Arc::new(MockStock {
        identity: StockIdentity {
            symbol: "AAPL".to_string(),
            exchange: None,
        },
        rx: Arc::new(tokio::sync::Mutex::new(rx)),
    });
    let market = Arc::new(MockMarket { stock: mock_stock });
    let trade = std::sync::Arc::new(crate::mock_trade::MockTradePort); let mut engine = JsEngine::new(market, trade);
    let captured = Arc::new(Mutex::new(Vec::new()));
    engine.register_handler(Box::new(MockHandler {
        captured: captured.clone(),
    }));

    let local = tokio::task::LocalSet::new();
    let handle = local.spawn_local(async move {
        engine
            .run_strategy("AAPL", "mock_account", TimeFrame::Minute1, &js_source)
            .await
    });

    local
        .run_until(async {
            // is_final=false → JS 策略直接返回 "null"
            tx.send(Candle {
                time: Utc::now(),
                open: 100.0,
                high: 155.0,
                low: 95.0,
                close: 150.0,
                adj_close: None,
                volume: 1000.0,
                is_final: false,
            })
            .unwrap();

            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            let signals = captured.lock().unwrap();
            assert_eq!(signals.len(), 0, "Non-final candle should NOT produce any signal");

            handle.abort();
        })
        .await;
}
