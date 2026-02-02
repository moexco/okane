use async_trait::async_trait;
use chrono::Utc;
use okane_core::common::{Stock as StockIdentity, TimeFrame};
use okane_core::engine::entity::{Signal, SignalKind};
use okane_core::engine::error::EngineError;
use okane_core::engine::port::SignalHandler;
use okane_core::market::entity::Candle;
use okane_core::market::error::MarketError;
use okane_core::market::port::{CandleStream, Market, Stock, StockStatus};
use okane_engine::runtime::WasmEngine;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

/// # Summary
/// 模拟股票聚合根。
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

    async fn fetch_history(&self, _: TimeFrame, _: usize) -> Result<Vec<Candle>, MarketError> {
        Ok(vec![])
    }
}

/// # Summary
/// 模拟市场服务。
struct MockMarket {
    stock: Arc<MockStock>,
}

#[async_trait]
impl Market for MockMarket {
    async fn get_stock(&self, _: &str) -> Result<Arc<dyn Stock>, MarketError> {
        Ok(self.stock.clone())
    }
}

/// # Summary
/// 模拟信号处理器，用于验证信号捕获。
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

#[tokio::test]
async fn test_wasm_strategy_integration() {
    // 1. 准备环境
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

    let mut engine = WasmEngine::new(market);
    let captured_signals = Arc::new(Mutex::new(Vec::new()));
    engine.register_handler(Box::new(MockHandler {
        captured: captured_signals.clone(),
    }));

    // 2. 加载编译好的 WASM
    let wasm_path = "../../target/wasm32-wasip1/debug/strategy_dummy.wasm";
    let wasm_bytes = std::fs::read(wasm_path).expect(
        "WASM file not found. Run 'cargo build --target wasm32-wasip1 -p strategy-dummy' first.",
    );

    // 3. 启动策略 (异步任务)
    let engine_arc = Arc::new(engine);
    let engine_clone = engine_arc.clone();

    let handle = tokio::spawn(async move {
        engine_clone
            .run_strategy("AAPL", TimeFrame::Minute1, &wasm_bytes)
            .await
    });

    // 4. 推送触发信号的数据 (close > 150.0)
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

    // 给一点处理时间
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // 5. 验证结果
    let signals = captured_signals.lock().unwrap();
    assert_eq!(signals.len(), 1, "Should have captured 1 signal");
    assert_eq!(signals[0].kind, SignalKind::LongEntry);
    assert_eq!(signals[0].strategy_id, "dummy-strategy");

    // 6. 停止测试 (由于 run_strategy 是无限循环，我们需要中止它)
    handle.abort();
}
