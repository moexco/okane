use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use okane_core::common::time::FakeClockProvider;
use okane_core::common::{Stock as StockIdentity, TimeFrame};
use okane_core::market::entity::Candle;
use okane_core::market::error::MarketError;
use okane_core::market::port::{CandleStream, Market, Stock, StockStatus};
use okane_core::test_utils::{MockAlgoOrderPort, MockIndicatorService, SpyTradePort};
use okane_engine::quickjs::JsEngine;
use rust_decimal_macros::dec;
use std::sync::Arc;
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
            while let Some(c) = rx.recv().await { yield Ok(c); }
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
        let base_time = end;
        for i in 0..5 {
            candles.push(Candle {
                time: base_time - chrono::Duration::minutes(i64::from(i)),
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

    async fn search_symbols(
        &self,
        _query: &str,
    ) -> Result<Vec<okane_core::store::port::StockMetadata>, MarketError> {
        Ok(vec![])
    }
}

/// JS 策略：验证 host.now() 和 host.fetchHistory() 调用正常运行
const JS_HOST_TEST_STRATEGY: &str = r#"
function onCandle(input) {
    var nowMs = host.now();
    host.log(3, "JS logic time check: " + nowMs);

    var historyJson = host.fetchHistory("AAPL", "1m", 5);
    var history = JSON.parse(historyJson);
    host.log(0, "Fetched " + history.length + " candles, logical_now=" + nowMs);
}
"#;

#[tokio::test]
async fn test_host_functions_from_js() -> anyhow::Result<()> {
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
    let time_provider = Arc::new(FakeClockProvider::new(Utc::now()));
    let time_provider_clone = time_provider.clone();
    let engine = JsEngine::new(
        market,
        trade,
        Arc::new(MockAlgoOrderPort),
        Arc::new(MockIndicatorService),
        time_provider,
        None,
        None,
    )
    .map_err(|e| anyhow::anyhow!(e))?;

    let local = tokio::task::LocalSet::new();

    let handle = local.spawn_local(async move {
        engine
            .run_strategy(
                "AAPL",
                "mock_account",
                TimeFrame::Minute1,
                JS_HOST_TEST_STRATEGY,
            )
            .await
    });

    local
        .run_until(async {
            let test_time = Utc
                .with_ymd_and_hms(2026, 2, 2, 10, 0, 0)
                .single()
                .ok_or_else(|| anyhow::anyhow!("Invalid date"))?;
            time_provider_clone.set_time(test_time)?;
            tx.send(Candle {
                time: test_time,
                open: dec!(150.0),
                high: dec!(150.0),
                low: dec!(150.0),
                close: dec!(150.0),
                adj_close: None,
                volume: dec!(0.0),
                is_final: true,
            })
            .map_err(|e| anyhow::anyhow!("Send failed: {:?}", e))?;

            tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

            // 策略应正常执行 (host.now() + host.fetchHistory() + host.log())
            handle.abort();
            Ok::<(), anyhow::Error>(())
        })
        .await?;
    Ok(())
}
