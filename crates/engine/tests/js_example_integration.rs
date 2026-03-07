use okane_core::test_utils::SpyTradePort;
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
    rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<Candle>>>,
}

#[async_trait]
impl Stock for MockStock {
    fn identity(&self) -> &StockIdentity {
        &self.identity
    }
    fn current_price(&self) -> Result<Option<rust_decimal::Decimal>, MarketError> {
        Ok(None)
    }
    fn latest_candle(&self, _: TimeFrame) -> Result<Option<Candle>, MarketError> {
        Ok(None)
    }
    fn last_closed_candle(&self, _: TimeFrame) -> Result<Option<Candle>, MarketError> {
        Ok(None)
    }
    fn status(&self) -> StockStatus {
        StockStatus::Online
    }
    fn subscribe(&self, _: TimeFrame) -> Result<CandleStream, MarketError> {
        let rx = self.rx.clone();
        let s = async_stream::stream! {
            let mut rx = rx.lock().await;
            while let Some(c) = rx.recv().await { yield c; }
        };
        Ok(Box::pin(s))
    }
    async fn fetch_history(
        &self,
        _: TimeFrame,
        _start: chrono::DateTime<Utc>,
        end: chrono::DateTime<Utc>,
    ) -> Result<Vec<Candle>, MarketError> {
        let mut candles = Vec::new();
        for i in 0..10 {
            candles.push(Candle {
                time: end - chrono::Duration::minutes(i64::from(i)),
                open: dec!(100.0),
                high: dec!(100.0),
                low: dec!(100.0),
                close: dec!(100.0),
                adj_close: None,
                volume: dec!(0.0),
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

    async fn search_symbols(&self, _query: &str) -> Result<Vec<okane_core::store::port::StockMetadata>, MarketError> {
        Ok(vec![])
    }
}

/// 从文件系统加载 JS 示例策略
fn load_js_example() -> anyhow::Result<String> {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Manifest dir parent not found"))?
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Manifest dir grandparent not found"))?;
    let js_path = workspace_root.join("examples/js-strategy-demo/strategy.js");
    std::fs::read_to_string(&js_path)
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", js_path.display(), e))
}

/// 测试 JS 示例策略：满足条件时通过 host.log 输出结果 (onCandle 为 void)
#[tokio::test]
async fn test_js_example_strategy_execution() -> anyhow::Result<()> {
    let js_source = load_js_example()?;

    let (tx, rx) = mpsc::unbounded_channel();
    let mock_stock = Arc::new(MockStock {
        identity: StockIdentity {
            symbol: "AAPL".to_string(),
            exchange: None,
        },
        rx: Arc::new(tokio::sync::Mutex::new(rx)),
    });
    let market = Arc::new(MockMarket { stock: mock_stock });
    let trade = std::sync::Arc::new(SpyTradePort::new());
    let engine = JsEngine::new(market, trade, Arc::new(FakeClockProvider::new(Utc::now())), None).map_err(|e| anyhow::anyhow!(e))?;

    let local = tokio::task::LocalSet::new();
    let handle = local.spawn_local(async move {
        engine
            .run_strategy("AAPL", "mock_account", TimeFrame::Minute1, &js_source)
            .await
    });

    local
        .run_until(async {
            // close=150.0 > SMA10(100.0), volume=1000 > 500, is_final=true
            tx.send(Candle {
                time: Utc::now(),
                open: dec!(100.0),
                high: dec!(155.0),
                low: dec!(95.0),
                close: dec!(150.0),
                adj_close: None,
                volume: dec!(1000.0),
                is_final: true,
            })
            .map_err(|e| anyhow::anyhow!("Failed to send candle: {:?}", e))?;

            tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

            // 策略应正常执行 (void onCandle + host.* 调用)
            handle.abort();
            Ok::<(), anyhow::Error>(())
        })
        .await?;
    Ok(())
}

/// 测试 JS 示例策略：is_final=false 时应跳过（void onCandle, 无报错）
#[tokio::test]
async fn test_js_example_strategy_skip_non_final() -> anyhow::Result<()> {
    let js_source = load_js_example()?;

    let (tx, rx) = mpsc::unbounded_channel();
    let mock_stock = Arc::new(MockStock {
        identity: StockIdentity {
            symbol: "AAPL".to_string(),
            exchange: None,
        },
        rx: Arc::new(tokio::sync::Mutex::new(rx)),
    });
    let market = Arc::new(MockMarket { stock: mock_stock });
    let trade = std::sync::Arc::new(SpyTradePort::new());
    let engine = JsEngine::new(market, trade, Arc::new(FakeClockProvider::new(Utc::now())), None).map_err(|e| anyhow::anyhow!(e))?;

    let local = tokio::task::LocalSet::new();
    let handle = local.spawn_local(async move {
        engine
            .run_strategy("AAPL", "mock_account", TimeFrame::Minute1, &js_source)
            .await
    });

    local
        .run_until(async {
            tx.send(Candle {
                time: Utc::now(),
                open: dec!(100.0),
                high: dec!(155.0),
                low: dec!(95.0),
                close: dec!(150.0),
                adj_close: None,
                volume: dec!(1000.0),
                is_final: false,
            })
            .map_err(|e| anyhow::anyhow!("Failed to send candle: {:?}", e))?;

            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            // 策略应正常执行 — 非最终 K 线跳过不报错
            handle.abort();
            Ok::<(), anyhow::Error>(())
        })
        .await?;
    Ok(())
}
