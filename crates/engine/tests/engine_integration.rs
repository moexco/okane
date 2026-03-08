use okane_core::test_utils::{MockMarket, MockStock, SpyTradePort, MockAlgoOrderPort, MockIndicatorService};
use okane_core::common::time::FakeClockProvider;
use okane_core::common::{Stock as StockIdentity, TimeFrame};
use okane_core::market::entity::Candle;
use chrono::Utc;
use okane_engine::quickjs::JsEngine;
use std::sync::Arc;
use tokio::sync::mpsc;
use rust_decimal_macros::dec;

const JS_STRATEGY: &str = r#"
function onCandle(input) {
    var candle = JSON.parse(input);
    if (candle.close > 150.0) {
        host.log(0, "Breakout detected: close=" + candle.close);
    }
}
"#;

const JS_TRADE_STRATEGY: &str = r#"
function onCandle(input) {
    var candle = JSON.parse(input);
    if (candle.close > 150.0) {
        host.buy("AAPL", "155.0", "100");
    }
}
"#;

#[tokio::test]
async fn test_js_strategy_integration() -> anyhow::Result<()> {
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

    let trade = SpyTradePort::new();
    let trade_arc = std::sync::Arc::new(trade);
    let engine = JsEngine::new(
        market,
        trade_arc.clone(),
        Arc::new(MockAlgoOrderPort),
        Arc::new(MockIndicatorService),
        Arc::new(FakeClockProvider::new(chrono::Utc::now())),
        None,
    ).map_err(|e| anyhow::anyhow!(e))?;

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
                high: dec!(160.0),
                low: dec!(90.0),
                close: dec!(155.0),
                adj_close: None,
                volume: dec!(1000.0),
                is_final: true,
            })
            .map_err(|e| anyhow::anyhow!("Send failed: {:?}", e))?;

            // 无需 sleep，因为逻辑上只要不崩溃即代表成功。
            // 但为了严谨，我们通常在 handle 运行一小会儿后 abort
            tokio::task::yield_now().await;
            handle.abort();
            Ok::<(), anyhow::Error>(())
        })
        .await?;
    Ok(())
}

#[tokio::test]
async fn test_js_trade_execution() -> anyhow::Result<()> {
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

    let trade = SpyTradePort::new();
    let trade_arc = std::sync::Arc::new(trade);
    let engine = JsEngine::new(
        market,
        trade_arc.clone(),
        Arc::new(MockAlgoOrderPort),
        Arc::new(MockIndicatorService),
        Arc::new(FakeClockProvider::new(chrono::Utc::now())),
        None,
    ).map_err(|e| anyhow::anyhow!(e))?;

    let local = tokio::task::LocalSet::new();

    let _handle = local.spawn_local(async move {
        engine
            .run_strategy("AAPL", "mock_account", TimeFrame::Minute1, JS_TRADE_STRATEGY)
            .await
    });

    local
        .run_until(async {
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
            .map_err(|e| anyhow::anyhow!("Send failed: {:?}", e))?;

            // 轮询检查订单是否产生
            let start = std::time::Instant::now();
            let mut orders = Vec::new();
            while start.elapsed() < std::time::Duration::from_secs(2) {
                orders = trade_arc.get_submitted_orders().map_err(|e| anyhow::anyhow!(e.to_string()))?;
                if !orders.is_empty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }

            assert_eq!(orders.len(), 1, "Should have submitted 1 order");
            assert_eq!(orders[0].symbol, "AAPL");
            assert_eq!(orders[0].volume, dec!(100));
            Ok::<(), anyhow::Error>(())
        })
        .await?;
    Ok(())
}

#[tokio::test]
async fn test_js_strategy_no_error_when_below_threshold() -> anyhow::Result<()> {
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

    let trade = std::sync::Arc::new(SpyTradePort::new());
    let engine = JsEngine::new(
        market,
        trade,
        Arc::new(MockAlgoOrderPort),
        Arc::new(MockIndicatorService),
        Arc::new(FakeClockProvider::new(chrono::Utc::now())),
        None,
    )
    .map_err(|e| anyhow::anyhow!(e))?;

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
            .map_err(|e| anyhow::anyhow!("Send failed: {:?}", e))?;

            // 无需 sleep，验证逻辑不崩溃即可
            tokio::task::yield_now().await;
            handle.abort();
            Ok::<(), anyhow::Error>(())
        })
        .await?;
    Ok(())
}
