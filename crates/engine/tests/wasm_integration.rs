use okane_core::test_utils::{MockMarket, MockStock, SpyTradePort};
use okane_core::common::time::FakeClockProvider;
use okane_core::common::{Stock as StockIdentity, TimeFrame};
use okane_core::market::entity::Candle;
use chrono::Utc;
use okane_engine::wasm::WasmEngine;
use std::sync::Arc;
use tokio::sync::mpsc;
use rust_decimal_macros::dec;

/// 构建 WASM 示例并返回字节码
fn build_wasm_example(package: &str) -> anyhow::Result<Vec<u8>> {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Parent not found"))?
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Grandparent not found"))?;

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
        .map_err(|e| anyhow::anyhow!("Failed to run cargo build: {}", e))?;

    if !status.success() {
        return Err(anyhow::anyhow!("WASM build failed for {}", package));
    }

    let wasm_name = package.replace('-', "_");
    let wasm_path = workspace_root
        .join("target/wasm32-unknown-unknown/release")
        .join(format!("{}.wasm", wasm_name));

    std::fs::read(&wasm_path)
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", wasm_path.display(), e))
}

#[tokio::test]
async fn test_wasm_strategy_dummy_execution() -> anyhow::Result<()> {
    let wasm_bytes = build_wasm_example("strategy-dummy")?;

    let (tx, rx) = mpsc::unbounded_channel();
    let mock_stock = Arc::new(MockStock {
        identity: StockIdentity {
            symbol: "AAPL".to_string(),
            exchange: None,
        },
        price_rx: Arc::new(tokio::sync::Mutex::new(rx)),
    });
    let market = Arc::new(MockMarket { stock: mock_stock });
    let trade = Arc::new(SpyTradePort::new());
    let engine = WasmEngine::new(market, trade.clone(), Arc::new(FakeClockProvider::new(chrono::Utc::now())), None).map_err(|e| anyhow::anyhow!(e))?;

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
    .map_err(|e| anyhow::anyhow!("Send failed: {:?}", e))?;

    // 轮询检查订单是否产生 (WASM 的 on_candle 逻辑应触发 host.buy)
    let start = std::time::Instant::now();
    let mut orders = Vec::new();
    while start.elapsed() < std::time::Duration::from_secs(3) {
        orders = trade.get_submitted_orders().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        if !orders.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    assert_eq!(orders.len(), 1, "WASM strategy should have submitted 1 order");
    assert_eq!(orders[0].symbol, "AAPL");

    handle.abort();
    Ok(())
}

#[tokio::test]
async fn test_wasm_strategy_dummy_below_threshold() -> anyhow::Result<()> {
    let wasm_bytes = build_wasm_example("strategy-dummy")?;

    let (tx, rx) = mpsc::unbounded_channel();
    let mock_stock = Arc::new(MockStock {
        identity: StockIdentity {
            symbol: "AAPL".to_string(),
            exchange: None,
        },
        price_rx: Arc::new(tokio::sync::Mutex::new(rx)),
    });
    let market = Arc::new(MockMarket { stock: mock_stock });
    let trade = Arc::new(SpyTradePort::new());
    let engine = WasmEngine::new(market, trade.clone(), Arc::new(FakeClockProvider::new(chrono::Utc::now())), None).map_err(|e| anyhow::anyhow!(e))?;

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
    .map_err(|e| anyhow::anyhow!("Send failed: {:?}", e))?;

    // 确认没有订单产生
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    let submitted = trade.get_submitted_orders().map_err(|e| anyhow::anyhow!(e.to_string()))?;
    assert!(submitted.is_empty(), "WASM strategy should NOT have submitted orders below threshold");

    handle.abort();
    Ok(())
}
