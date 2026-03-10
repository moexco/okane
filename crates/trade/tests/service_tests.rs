use okane_core::common::{Stock as StockIdentity, TimeFrame};
use okane_core::market::entity::Candle;
use okane_core::market::error::MarketError;
use okane_core::market::port::{CandleStream, Market, Stock, StockStatus};
use okane_core::trade::entity::{AccountId, Order, OrderDirection, OrderId, AccountSnapshot};
use okane_core::trade::port::TradePort;
use okane_trade::account::AccountManager;
use okane_trade::service::TradeService;
use rust_decimal_macros::dec;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::time::{sleep, Duration};

struct DummyStock {
    identity: StockIdentity,
}

#[async_trait::async_trait]
impl Stock for DummyStock {
    fn identity(&self) -> &StockIdentity {
        &self.identity
    }
    fn current_price(&self) -> Result<Option<rust_decimal::Decimal>, MarketError> {
        Ok(Some(dec!(150.0))) // Mock 固定的市场价用于本地测试撮合
    }
    fn latest_candle(&self, _timeframe: TimeFrame) -> Result<Option<Candle>, MarketError> { Ok(None) }
    fn last_closed_candle(&self, _timeframe: TimeFrame) -> Result<Option<Candle>, MarketError> { Ok(None) }
    fn subscribe(&self, _timeframe: TimeFrame) -> Result<CandleStream, MarketError> { unimplemented!() }
    async fn fetch_history(
        &self,
        _timeframe: TimeFrame,
        _start: chrono::DateTime<chrono::Utc>,
        _end: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<Candle>, MarketError> {
        unimplemented!()
    }
    
    fn status(&self) -> StockStatus { StockStatus::Online }
}

struct MockMarket;

#[async_trait::async_trait]
impl Market for MockMarket {
    async fn get_stock(&self, symbol: &str) -> Result<std::sync::Arc<dyn Stock>, MarketError> {
        Ok(Arc::new(DummyStock {
            identity: StockIdentity {
                symbol: symbol.to_string(),
                exchange: None,
            },
        }))
    }

    async fn search_symbols(&self, _query: &str) -> Result<Vec<okane_core::store::port::StockMetadata>, MarketError> {
        Ok(vec![])
    }
}

#[tokio::test]
#[allow(clippy::manual_is_multiple_of)]
async fn test_high_concurrency_order_execution() -> anyhow::Result<()> {
    let account_manager = Arc::new(AccountManager::new());
    let acct_id = AccountId("TestWallet_01".to_string());
    
    // 初始化可用余额：1,000,000.00
    account_manager.ensure_account_exists(acct_id.clone(), dec!(1000000.0));
    
    let market = Arc::new(MockMarket);
    let pending_port = Arc::new(okane_store::pending_order::MemoryPendingOrderStore::new());
    // 用 Arc 包裹 TradeService 供多线程闭包移动
    let matcher = std::sync::Arc::new(okane_trade::matcher::LocalMatchEngine::new(rust_decimal::Decimal::from_str_exact("0.0001").map_err(|e| anyhow::anyhow!(e))?));
    let trade_service = Arc::new(TradeService::new(account_manager.clone(), matcher, market, pending_port, Arc::new(okane_core::common::time::RealTimeProvider)));
    
    let mut handles = vec![];
    let counter = Arc::new(AtomicUsize::new(0));

    // 开启买单并发轰炸
    for _ in 0..100 {
        let ts = trade_service.clone();
        let aid = acct_id.clone();
        let c = counter.clone();
        handles.push(tokio::spawn(async move {
            let i = c.fetch_add(1, Ordering::SeqCst);
            let order = Order::new(
                OrderId(format!("BUY_{}", i)),
                aid,
                "AAPL".to_string(),
                OrderDirection::Buy,
                None, // 修改为市价单以触发立即撮合
                dec!(10.0),
                0,
            );
            // 稍作打乱执行时序
            if i % 3 == 0 {
                sleep(Duration::from_millis(1)).await;
            }
            ts.submit_order(order).await.map_err(|e| anyhow::anyhow!("Submit error: {:?}", e))?;
            Ok::<(), anyhow::Error>(())
        }));
    }

    // 开启卖单并发轰炸
    for _ in 0..50 {
        let ts = trade_service.clone();
        let aid = acct_id.clone();
        let c = counter.clone();
        handles.push(tokio::spawn(async move {
            let i = c.fetch_add(1, Ordering::SeqCst);
            let order = Order::new(
                OrderId(format!("SELL_{}", i)),
                aid,
                "AAPL".to_string(),
                OrderDirection::Sell,
                None, // fallback to current price in mock
                dec!(10.0),
                0,
            );
            ts.submit_order(order).await.map_err(|e| anyhow::anyhow!("Submit error: {:?}", e))?;
            Ok::<(), anyhow::Error>(())
        }));
    }

    for h in handles {
        h.await.map_err(|e| anyhow::anyhow!("Join error: {}", e))??;
    }

    // 全量核对状态
    let snapshot: AccountSnapshot = trade_service.get_account(acct_id).await.map_err(|e| anyhow::anyhow!(e))?;
    
    assert_eq!(snapshot.frozen_balance, dec!(0.0), "all frozen balance must be released after heavy concurrency");
    assert_eq!(snapshot.available_balance, dec!(924977.5), "balance transfer must be consistent without loss");
    
    assert_eq!(snapshot.positions.len(), 1);
    let pos = snapshot.positions.first().ok_or_else(|| anyhow::anyhow!("AAPL position should exist"))?;
    assert_eq!(pos.symbol, "AAPL");
    assert_eq!(pos.volume, dec!(500.0), "aapl net long position must be 500");
    Ok(())
}

#[tokio::test]
async fn test_insufficient_funds_rejection() -> anyhow::Result<()> {
    let account_manager = Arc::new(AccountManager::new());
    let acct_id = AccountId("PoorWallet".to_string());
    
    // 只有 $10
    account_manager.ensure_account_exists(acct_id.clone(), dec!(10.0));
    
    let market = Arc::new(MockMarket);
    let pending_port = Arc::new(okane_store::pending_order::MemoryPendingOrderStore::new());
    let matcher = std::sync::Arc::new(okane_trade::matcher::LocalMatchEngine::new(rust_decimal::Decimal::ZERO));
    let trade_service = Arc::new(TradeService::new(account_manager.clone(), matcher, market, pending_port, Arc::new(okane_core::common::time::RealTimeProvider)));
    
    // 购买 1 股 AAPL, 价格 150
    let order = Order::new(
        OrderId("B_1".into()),
        acct_id.clone(),
        "AAPL".into(),
        OrderDirection::Buy,
        None,
        dec!(1.0),
        0
    );

    let res = trade_service.submit_order(order).await;
    assert!(res.is_err(), "insufficient funds order must be rejected");

    match res.err().ok_or_else(|| anyhow::anyhow!("Expected error"))? {
        okane_core::trade::port::TradeError::InsufficientFunds { .. } => {}
        _ => return Err(anyhow::anyhow!("unexpected error type")),
    }
    
    // 断言资金安全
    let snapshot: AccountSnapshot = trade_service.get_account(acct_id).await.map_err(|e| anyhow::anyhow!(e))?;
    assert_eq!(snapshot.available_balance, dec!(10.0));
    assert_eq!(snapshot.frozen_balance, dec!(0.0));
    assert!(snapshot.positions.is_empty());
    Ok(())
}
