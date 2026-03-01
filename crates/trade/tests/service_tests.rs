use okane_core::common::{Stock as StockIdentity, TimeFrame};
use okane_core::market::entity::Candle;
use okane_core::market::error::MarketError;
use okane_core::market::port::{Market, Stock, StockStatus};
use okane_core::trade::entity::{AccountId, Order, OrderDirection, OrderId};
use okane_core::trade::port::TradePort;
use okane_trade::account::AccountManager;
use okane_trade::service::TradeService;
use rust_decimal_macros::dec;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};

struct DummyStock {
    identity: StockIdentity,
}

#[async_trait::async_trait]
impl Stock for DummyStock {
    fn identity(&self) -> &StockIdentity {
        &self.identity
    }
    fn current_price(&self) -> Option<f64> {
        Some(150.0) // Mock 固定的市场价用于本地测试撮合
    }
    fn latest_candle(&self, _timeframe: TimeFrame) -> Option<Candle> { None }
    fn last_closed_candle(&self, _timeframe: TimeFrame) -> Option<Candle> { None }
    fn subscribe(&self, _timeframe: TimeFrame) -> okane_core::market::port::CandleStream { unimplemented!() }
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
async fn test_high_concurrency_order_execution() {
    let account_manager = Arc::new(AccountManager::new());
    let acct_id = AccountId("TestWallet_01".to_string());
    
    // 初始化可用余额：1,000,000.00
    account_manager.ensure_account_exists(acct_id.clone(), dec!(1000000.0));
    
    let market = Arc::new(MockMarket);
    let pending_port = Arc::new(okane_store::pending_order::MemoryPendingOrderStore::new());
    // 用 Arc 包裹 TradeService 供多线程闭包移动
    let matcher = std::sync::Arc::new(okane_trade::matcher::LocalMatchEngine::new(rust_decimal::Decimal::from_str_exact("0.0001").unwrap()));
    let trade_service = Arc::new(TradeService::new(account_manager.clone(), matcher, market, pending_port));
    
    // 测试：并发抛入 100 张限价买单和 50 张市价卖单。
    // 单张买单需要金额: volume(10) * price(150) = 1500
    // 100 张买单总额度: 150,000. 手续费(0.0001): 15. 总花费: 150,015.
    // 单张卖单收获: 10 * 150 - 手续费(0.15) = 1499.85
    // 50 张卖单总收获: 74,992.5
    // 测试终态可用余额应为: 1,000,000 - 150,015 + 74,992.5 = 924,977.5
    // 测试总持仓应为: 100 * 10 - 50 * 10 = 500
    
    let mut handles: Vec<JoinHandle<()>> = vec![];
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
                None, // 修改为市价单以触发立即撮合，修复原版遗留 Bug
                dec!(10.0),
                0,
            );
            // 稍作打乱执行时序
            if i.is_multiple_of(3) {
                sleep(Duration::from_millis(1)).await;
            }
            let res = ts.submit_order(order).await;
            assert!(res.is_ok());
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
            let res = ts.submit_order(order).await;
            assert!(res.is_ok());
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    // 全量核对状态
    let snapshot = trade_service.get_account(acct_id).await.unwrap();
    
    assert_eq!(snapshot.frozen_balance, dec!(0.0), "并发后所有冻结资金应被清盘平出");
    assert_eq!(snapshot.available_balance, dec!(924977.5), "资金划转必须满足读写一致无丢失");
    
    assert_eq!(snapshot.positions.len(), 1);
    let pos = snapshot.positions.first().unwrap();
    assert_eq!(pos.symbol, "AAPL");
    assert_eq!(pos.volume, dec!(500.0), "苹果股票净多头持仓应为 500");
}

#[tokio::test]
async fn test_insufficient_funds_rejection() {
    let account_manager = Arc::new(AccountManager::new());
    let acct_id = AccountId("PoorWallet".to_string());
    
    // 只有 $10
    account_manager.ensure_account_exists(acct_id.clone(), dec!(10.0));
    
    let market = Arc::new(MockMarket);
    let pending_port = Arc::new(okane_store::pending_order::MemoryPendingOrderStore::new());
    let matcher = std::sync::Arc::new(okane_trade::matcher::LocalMatchEngine::new(rust_decimal::Decimal::ZERO));
    let trade_service = Arc::new(TradeService::new(account_manager.clone(), matcher, market, pending_port));
    
    // 购买 1 股 AAPL, 价格 150，理应风控拒绝并无法入队撮合
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
    assert!(res.is_err(), "金额不足订单未被拒绝");

    match res.unwrap_err() {
        okane_core::trade::port::TradeError::InsufficientFunds { .. } => {}
        _ => panic!("错误类型不符"),
    }
    
    // 断言资金安全
    let snapshot = trade_service.get_account(acct_id).await.unwrap();
    assert_eq!(snapshot.available_balance, dec!(10.0));
    assert_eq!(snapshot.frozen_balance, dec!(0.0));
    assert!(snapshot.positions.is_empty());
}
