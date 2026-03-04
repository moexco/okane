pub mod mock_trade;
use async_trait::async_trait;
use chrono::Utc;
use okane_core::common::time::FakeClockProvider;
use okane_core::common::{Stock as StockIdentity, TimeFrame};
use okane_core::market::entity::Candle;
use okane_core::market::error::MarketError;
use okane_core::market::port::{CandleStream, Market, Stock, StockStatus};
use okane_engine::wasm::WasmEngine;
use std::sync::Arc;
use tokio::sync::mpsc;
use rust_decimal_macros::dec;

struct MockStock {
    identity: StockIdentity,
    rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<Candle>>>,
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

/// 构建 WASM 示例并返回字节码路径
fn build_wasm_example(package: &str) -> Vec<u8> {
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

    let wasm_name = package.replace('-', "_");
    let wasm_path = workspace_root
        .join("target/wasm32-unknown-unknown/release")
        .join(format!("{}.wasm", wasm_name));

    std::fs::read(&wasm_path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {}", wasm_path.display(), e))
}

#[tokio::test]
async fn test_wasm_strategy_dummy_execution() {
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
    let trade = std::sync::Arc::new(crate::mock_trade::MockTradePort);
    let engine = WasmEngine::new(market, trade, Arc::new(FakeClockProvider::new(Utc::now())), None);

    let handle = tokio::spawn(async move {
        engine
            .run_strategy("AAPL", "mock_account", TimeFrame::Minute1, &wasm_bytes)
            .await
    });

    // close=155.0 > 150.0
    tx.send(Candle {
        time: Utc::now(),
        open: dec!(150.0),
        high: dec!(160.0),
        low: dec!(140.0),
        close: dec!(155.0),
        adj_close: None,
        volume: dec!(1000.0),
        is_final: true,
    })
    .unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // 策略应正常执行 (void on_candle)
    handle.abort();
}

#[tokio::test]
async fn test_wasm_strategy_dummy_below_threshold() {
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
    let trade = std::sync::Arc::new(crate::mock_trade::MockTradePort);
    let engine = WasmEngine::new(market, trade, Arc::new(FakeClockProvider::new(Utc::now())), None);

    let handle = tokio::spawn(async move {
        engine
            .run_strategy("AAPL", "mock_account", TimeFrame::Minute1, &wasm_bytes)
            .await
    });

    // close=100.0 <= 150.0 → 不应触发任何动作
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

    // 策略应正常执行 — 不触发动作不报错
    handle.abort();
}
