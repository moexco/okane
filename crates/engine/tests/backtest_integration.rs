use chrono::{TimeZone, Utc};
use okane_core::common::time::FakeClockProvider;
use okane_core::common::{Stock as StockIdentity, TimeFrame};
use okane_core::market::entity::Candle;
use okane_core::market::error::MarketError;
use okane_core::market::port::{CandleStream, Market, Stock, StockStatus};
use okane_core::trade::entity::AccountId;
use okane_core::trade::port::AccountPort;
use okane_engine::backtest::BacktestDriver;

use okane_trade::account::AccountManager;
use okane_trade::service::TradeService;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::sync::Arc;
use tokio::sync::mpsc;
use async_trait::async_trait;

struct HistoricalMockStock {
    identity: StockIdentity,
    history: Vec<Candle>,
}

#[async_trait]
impl Stock for HistoricalMockStock {
    fn identity(&self) -> &StockIdentity {
        &self.identity
    }
    fn current_price(&self) -> Result<Option<Decimal>, MarketError> {
        Ok(self.history.last().map(|c| c.close))
    }
    fn latest_candle(&self, _: TimeFrame) -> Result<Option<Candle>, MarketError> { Ok(None) }
    fn last_closed_candle(&self, _: TimeFrame) -> Result<Option<Candle>, MarketError> { Ok(None) }
    fn status(&self) -> StockStatus { StockStatus::Online }
    fn subscribe(&self, _: TimeFrame) -> Result<CandleStream, MarketError> {
        let (_tx, rx) = mpsc::unbounded_channel::<Candle>();
        let s = async_stream::stream! {
            let mut rx = rx;
            while let Some(c) = rx.recv().await { yield c; }
        };
        Ok(Box::pin(s))
    }
    async fn fetch_history(
        &self,
        _: TimeFrame,
        _: chrono::DateTime<Utc>,
        _: chrono::DateTime<Utc>,
    ) -> Result<Vec<Candle>, MarketError> {
        Ok(self.history.clone())
    }
}

struct HistoricalMarket {
    stock: Arc<HistoricalMockStock>,
}

#[async_trait]
impl Market for HistoricalMarket {
    async fn get_stock(&self, _: &str) -> Result<Arc<dyn Stock>, MarketError> {
        Ok(self.stock.clone())
    }

    async fn search_symbols(&self, _query: &str) -> Result<Vec<okane_core::store::port::StockMetadata>, MarketError> {
        Ok(vec![])
    }
}


#[tokio::test]
async fn test_end_to_end_backtest_with_time_travel() -> anyhow::Result<()> {
    let base_time = Utc.with_ymd_and_hms(2025, 1, 1, 9, 30, 0)
        .single()
        .ok_or_else(|| anyhow::anyhow!("Invalid date"))?;
    
    // 造 3 根 K 线:
    // T0: 开盘 100，收盘 110
    // T1: 开盘 110，最高 150，收盘 120
    // T2: 开盘 120，最高 190 (将击穿止盈限价单)，收盘 180
    let history = vec![
        Candle { time: base_time, open: dec!(100.0), high: dec!(110.0), low: dec!(100.0), close: dec!(110.0), adj_close: None, volume: dec!(1000.0), is_final: true },
        Candle { time: base_time + chrono::Duration::minutes(1), open: dec!(110.0), high: dec!(150.0), low: dec!(110.0), close: dec!(120.0), adj_close: None, volume: dec!(1000.0), is_final: true },
        Candle { time: base_time + chrono::Duration::minutes(2), open: dec!(120.0), high: dec!(190.0), low: dec!(120.0), close: dec!(180.0), adj_close: None, volume: dec!(1000.0), is_final: true },
    ];
    
    let mock_stock = Arc::new(HistoricalMockStock {
        identity: StockIdentity { symbol: "VOO".to_string(), exchange: None },
        history: history.clone(),
    });
    
    let market = Arc::new(HistoricalMarket { stock: mock_stock });
    let account_manager = Arc::new(AccountManager::new());
    
    let account_id = AccountId("test-account".to_string());
    account_manager.ensure_account_exists(account_id.clone(), Decimal::from_str_exact("10000.0").map_err(|e| anyhow::anyhow!(e))?);

    let pending_port = Arc::new(okane_store::pending_order::MemoryPendingOrderStore::new());
    let matcher = std::sync::Arc::new(okane_trade::matcher::LocalMatchEngine::new(Decimal::ZERO));
    let fake_clock = Arc::new(FakeClockProvider::new(base_time));
    let trade_service = Arc::new(TradeService::new(account_manager.clone(), matcher, market.clone(), pending_port, fake_clock.clone()));

    // 创建 BacktestDriver
    let driver = BacktestDriver::new(market.clone(), trade_service.clone(), fake_clock.clone());
    
    // 运行整个回测
    driver.run("VOO", TimeFrame::Minute1, base_time, 3).await.map_err(|e| anyhow::anyhow!(e))?;


    // 检查最终账户流水状态
    let snapshot = account_manager.snapshot(&account_id).await.map_err(|e| anyhow::anyhow!(e))?;
    tracing::info!("Final snapshot: {:?}", snapshot);
    Ok(())
}
