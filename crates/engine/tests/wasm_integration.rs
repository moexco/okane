use async_trait::async_trait;
use chrono::Utc;
use okane_core::common::{Stock as StockIdentity, TimeFrame};
use okane_core::engine::entity::{Signal, SignalKind};
use okane_core::engine::error::EngineError;
use okane_core::engine::port::SignalHandler;
use okane_core::market::entity::Candle;
use okane_core::market::error::MarketError;
use okane_core::market::port::{CandleStream, Market, Stock, StockStatus};
use okane_engine::wasm::WasmEngine;
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

/// 构建 WASM 示例并返回字节码路径
fn build_wasm_example(package: &str) -> Vec<u8> {
    // 先构建
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();

    let status = std::process::Command::new("cargo")
        .args([
            "build",
            "--target",
            "wasm32-unknown-unknown",
            "-p",
            package,
            "--release",
        ])
        .current_dir(workspace_root)
        .status()
        .expect("Failed to run cargo build for WASM example");

    assert!(status.success(), "WASM build failed for {}", package);

    // 读取产物
    let wasm_name = package.replace('-', "_");
    let wasm_path = workspace_root
        .join("target/wasm32-unknown-unknown/release")
        .join(format!("{}.wasm", wasm_name));

    std::fs::read(&wasm_path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {}", wasm_path.display(), e))
}

#[tokio::test]
async fn test_wasm_strategy_dummy_signal() {
    let wasm_bytes = build_wasm_example("strategy-dummy");

    let (tx, rx) = mpsc::unbounded_channel();
    let mock_stock = Arc::new(MockStock {
        identity: StockIdentity {
            symbol: "AAPL".to_string(),
            exchange: None,
        },
        rx: Arc::new(tokio::sync::Mutex::new(rx)),
    });
    let market = Arc::new(MockMarket { stock: mock_stock });
    let mut engine = WasmEngine::new(market);
    let captured = Arc::new(Mutex::new(Vec::new()));
    engine.register_handler(Box::new(MockHandler {
        captured: captured.clone(),
    }));

    let handle = tokio::spawn(async move {
        engine
            .run_strategy("AAPL", TimeFrame::Minute1, &wasm_bytes)
            .await
    });

    // close=155.0 > 150.0 → 应触发 LongEntry
    tx.send(Candle {
        time: Utc::now(),
        open: 150.0,
        high: 160.0,
        low: 140.0,
        close: 155.0,
        adj_close: None,
        volume: 1000.0,
        is_final: true,
    })
    .unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let signals = captured.lock().unwrap();
    assert_eq!(signals.len(), 1, "Should have captured 1 signal from WASM");
    assert_eq!(signals[0].kind, SignalKind::LongEntry);
    assert_eq!(signals[0].strategy_id, "wasm-dummy");

    handle.abort();
}

#[tokio::test]
async fn test_wasm_strategy_dummy_no_signal() {
    let wasm_bytes = build_wasm_example("strategy-dummy");

    let (tx, rx) = mpsc::unbounded_channel();
    let mock_stock = Arc::new(MockStock {
        identity: StockIdentity {
            symbol: "AAPL".to_string(),
            exchange: None,
        },
        rx: Arc::new(tokio::sync::Mutex::new(rx)),
    });
    let market = Arc::new(MockMarket { stock: mock_stock });
    let mut engine = WasmEngine::new(market);
    let captured = Arc::new(Mutex::new(Vec::new()));
    engine.register_handler(Box::new(MockHandler {
        captured: captured.clone(),
    }));

    let handle = tokio::spawn(async move {
        engine
            .run_strategy("AAPL", TimeFrame::Minute1, &wasm_bytes)
            .await
    });

    // close=100.0 <= 150.0 → 不应触发信号
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

    let signals = captured.lock().unwrap();
    assert_eq!(signals.len(), 0, "Should NOT have captured any signal");

    handle.abort();
}
