//! # 测试工具集 (Test Utilities)
//! 
//! 本模块提供统一的 Mock 实现和测试辅助工具，仅在启用 `test-utils` 特性时可用。
//! 这些工具被设计为跨模块通用，以消除测试代码中的逻辑重复。

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use crate::common::{Stock as StockIdentity, TimeFrame};
use crate::market::entity::Candle;
use crate::market::error::MarketError;
use crate::market::port::{CandleStream, MarketDataProvider, StockStatus};
use crate::store::error::StoreError;
use crate::store::port::{MarketStore, StockMetadata};
use crate::trade::entity::{AccountId, AccountSnapshot, Order, OrderId};
use crate::trade::port::{TradeError, TradePort};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

// ============================================================
//  行情驱动 Mock (Market Data Provider Mock)
// ============================================================

pub struct MockMarketDataProvider {
    price_tx: mpsc::UnboundedSender<Candle>,
    price_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<Candle>>>,
    search_results: Arc<Mutex<Vec<StockMetadata>>>,
    history: Arc<Mutex<Vec<Candle>>>,
}

impl Default for MockMarketDataProvider {
    fn default() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            price_tx: tx,
            price_rx: Arc::new(tokio::sync::Mutex::new(rx)),
            search_results: Arc::new(Mutex::new(Vec::new())),
            history: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl MockMarketDataProvider {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_candle(&self, candle: Candle) {
        if let Err(e) = self.price_tx.send(candle) {
            tracing::warn!("MockMarketDataProvider: failed to push candle: {}", e);
        }
    }

    pub fn set_search_results(&self, results: Vec<StockMetadata>) -> Result<(), crate::error::CoreError> {
        let mut guard = self.search_results.lock().map_err(|e| crate::error::CoreError::Poisoned(e.to_string()))?;
        *guard = results;
        Ok(())
    }

    pub fn set_history(&self, candles: Vec<Candle>) -> Result<(), crate::error::CoreError> {
        let mut guard = self.history.lock().map_err(|e| crate::error::CoreError::Poisoned(e.to_string()))?;
        *guard = candles;
        Ok(())
    }
}

pub struct MockStock {
    pub identity: StockIdentity,
    pub price_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<Candle>>>,
}

#[async_trait]
impl crate::market::port::Stock for MockStock {
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
        let rx = self.price_rx.clone();
        let s = async_stream::stream! {
            let mut rx = rx.lock().await;
            while let Some(candle) = rx.recv().await {
                yield candle;
            }
        };
        Ok(box_pin_stream(s))
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

fn box_pin_stream(s: impl futures::Stream<Item = Candle> + Send + 'static) -> CandleStream {
    Box::pin(s)
}

pub struct MockMarket {
    pub stock: Arc<dyn crate::market::port::Stock>,
}

#[async_trait]
impl crate::market::port::Market for MockMarket {
    async fn get_stock(&self, _: &str) -> Result<Arc<dyn crate::market::port::Stock>, MarketError> {
        Ok(self.stock.clone())
    }

    async fn search_symbols(&self, _query: &str) -> Result<Vec<StockMetadata>, MarketError> {
        Ok(vec![])
    }
}

#[async_trait]
impl MarketDataProvider for MockMarketDataProvider {
    async fn fetch_candles(
        &self,
        _: &StockIdentity,
        _: TimeFrame,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Candle>, MarketError> {
        let history = self.history.lock().map_err(|e| MarketError::Unknown(format!("Lock poisoned: {}", e)))?.clone();
        Ok(history.into_iter().filter(|c| c.time >= start && c.time <= end).collect())
    }

    async fn subscribe_candles(
        &self,
        _: &StockIdentity,
        _: TimeFrame,
    ) -> Result<CandleStream, MarketError> {
        let rx = self.price_rx.clone();
        let s = async_stream::stream! {
            let mut rx = rx.lock().await;
            while let Some(candle) = rx.recv().await {
                yield candle;
            }
        };
        Ok(Box::pin(s))
    }

    async fn search_symbols(&self, _query: &str) -> Result<Vec<StockMetadata>, MarketError> {
        let results = self.search_results.lock().map_err(|e| MarketError::Unknown(format!("Lock poisoned: {}", e)))?.clone();
        Ok(results)
    }
}

// ============================================================
//  内存行情存储 (In-Memory Market Store)
// ============================================================

pub struct MemMarketStore {
    db: dashmap::DashMap<(String, TimeFrame), Vec<Candle>>,
}

impl Default for MemMarketStore {
    fn default() -> Self {
        Self { db: dashmap::DashMap::new() }
    }
}

impl MemMarketStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl MarketStore for MemMarketStore {
    async fn save_candles(
        &self,
        stock: &StockIdentity,
        timeframe: TimeFrame,
        candles: &[Candle],
    ) -> Result<(), StoreError> {
        let mut entry = self.db.entry((stock.symbol.clone(), timeframe)).or_default();
        entry.extend_from_slice(candles);
        Ok(())
    }

    async fn load_candles(
        &self,
        stock: &StockIdentity,
        timeframe: TimeFrame,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Candle>, StoreError> {
        // 内存存储 Mock 逻辑：如果 key 不存在，返回空列表是合规的初始化状态。
        // OK: Mock store fallback
        let candles = self.db.get(&(stock.symbol.clone(), timeframe))
            .map(|v| v.clone())
            .unwrap_or_default();
        Ok(candles.into_iter().filter(|c| c.time >= start && c.time <= end).collect())
    }
}

// ============================================================
//  交易接口 Spy (Trade Port Spy)
// ============================================================

pub struct SpyTradePort {
    submitted_orders: Arc<Mutex<Vec<Order>>>,
}

impl Default for SpyTradePort {
    fn default() -> Self {
        Self {
            submitted_orders: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl SpyTradePort {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_submitted_orders(&self) -> Result<Vec<Order>, crate::error::CoreError> {
        Ok(self.submitted_orders.lock().map_err(|e| crate::error::CoreError::Poisoned(format!("SpyTradePort lock error: {}", e)))?.clone())
    }
}

#[async_trait]
impl TradePort for SpyTradePort {
    async fn submit_order(&self, order: Order) -> Result<OrderId, TradeError> {
        let id = OrderId(uuid::Uuid::new_v4().to_string());
        let mut orders = self.submitted_orders.lock().map_err(|e| TradeError::InternalError(format!("Lock poisoned: {}", e)))?;
        orders.push(order);
        Ok(id)
    }

    async fn cancel_order(&self, _id: OrderId) -> Result<(), TradeError> {
        Ok(())
    }

    async fn get_account(&self, _account_id: AccountId) -> Result<AccountSnapshot, TradeError> {
        Ok(AccountSnapshot::default())
    }

    async fn get_orders(&self, _account_id: &AccountId) -> Result<Vec<Order>, TradeError> {
        let orders = self.submitted_orders.lock().map_err(|e| TradeError::InternalError(format!("Lock poisoned: {}", e)))?;
        Ok(orders.clone())
    }

    async fn get_order(&self, _order_id: &OrderId) -> Result<Option<Order>, TradeError> {
        Ok(None)
    }
}

// ============================================================
//  算法单 Mock (Algo Order Mock)
// ============================================================

pub struct MockAlgoOrderPort;

#[async_trait]
impl crate::trade::port::AlgoOrderPort for MockAlgoOrderPort {
    async fn submit_algo_order(&self, _order: crate::trade::entity::AlgoOrder) -> Result<OrderId, TradeError> {
        Ok(OrderId(uuid::Uuid::new_v4().to_string()))
    }
    async fn cancel_algo_order(&self, _id: &OrderId) -> Result<(), TradeError> {
        Ok(())
    }
    async fn get_algo_order(&self, _id: &OrderId) -> Result<Option<crate::trade::entity::AlgoOrder>, TradeError> {
        Ok(None)
    }
    async fn get_algo_orders(&self, _account_id: &AccountId) -> Result<Vec<crate::trade::entity::AlgoOrder>, TradeError> {
        Ok(vec![])
    }
    async fn update_algo_status(&self, _id: &OrderId, _status: crate::trade::entity::AlgoOrderStatus) -> Result<(), TradeError> {
        Ok(())
    }
}

// ============================================================
//  指标服务 Mock (Indicator Service Mock)
// ============================================================

pub struct MockIndicatorService;

#[async_trait]
impl crate::market::indicator::IndicatorService for MockIndicatorService {
    async fn sma(&self, _symbol: &str, _tf: TimeFrame, _period: u32) -> Result<rust_decimal::Decimal, MarketError> {
        Ok(rust_decimal::Decimal::ZERO)
    }
    async fn ema(&self, _symbol: &str, _tf: TimeFrame, _period: u32) -> Result<rust_decimal::Decimal, MarketError> {
        Ok(rust_decimal::Decimal::ZERO)
    }
    async fn rsi(&self, _symbol: &str, _tf: TimeFrame, _period: u32) -> Result<rust_decimal::Decimal, MarketError> {
        Ok(rust_decimal::Decimal::ZERO)
    }
}

// ============================================================
//  测试辅助函数 (Test Helpers)
// ============================================================

/// # Summary
/// 轮询等待直到满足特定条件或超时。
pub async fn wait_for_condition<F, Fut>(
    timeout: std::time::Duration,
    interval: std::time::Duration,
    condition: F,
) -> bool
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if condition().await {
            return true;
        }
        tokio::time::sleep(interval).await;
    }
    false
}
