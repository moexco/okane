//! # 回测市场聚合根
//!
//! 实现 `BacktestMarket` 和 `BacktestStock`，作为回测限界上下文的核心组件。
//!
//! ## DDD 设计理念
//! 在回测场景中，"市场"不仅仅是数据源——它是整个模拟世界的聚合根：
//! - **时间**由它决定（通过 `FakeClockProvider`）
//! - **撮合**由它触发（通过 `BacktestTradePort.tick()`）
//! - **数据**由它控制（严格按时间截断，防止泄露未来信息）
//!
//! 策略和引擎完全不感知自己运行在回测环境中。

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use okane_core::common::time::{FakeClockProvider, TimeProvider};
use okane_core::common::{Stock as StockIdentity, TimeFrame};
use okane_core::market::entity::Candle;
use okane_core::market::error::MarketError;
use okane_core::market::port::{CandleStream, Market, Stock, StockStatus};
use okane_core::trade::port::BacktestTradePort;
use rust_decimal::Decimal;
use std::sync::Arc;
use tracing::debug;

/// # Summary
/// 回测专用的 Stock 聚合根。
///
/// # Invariants
/// - `subscribe()` 返回的 stream 在每次 yield K 线前自动推进时钟和驱动撮合。
/// - `current_price()` 根据 `time_provider.now()` 查找对应的 K 线收盘价。
/// - `fetch_history()` 严格按 `end` 参数截断数据，防止泄露未来信息。
pub struct BacktestStock {
    identity: StockIdentity,
    /// 按时间升序排列的完整历史 K 线数据
    candles: Vec<Candle>,
    /// 回测时钟，由 BacktestStock 在每次 yield 时推进
    time_provider: Arc<FakeClockProvider>,
    /// 撮合端口，在每根 K 线到达时调用 tick 驱动限价单撮合
    trade_port: Arc<dyn BacktestTradePort>,
}

impl BacktestStock {
    /// 创建回测用 Stock 聚合根
    ///
    /// # Arguments
    /// * `symbol` - 证券代码
    /// * `candles` - 按时间升序排列的历史 K 线数据
    /// * `time_provider` - 回测时钟（FakeClockProvider）
    /// * `trade_port` - 支持 tick() 的撮合端口
    pub fn new(
        symbol: String,
        candles: Vec<Candle>,
        time_provider: Arc<FakeClockProvider>,
        trade_port: Arc<dyn BacktestTradePort>,
    ) -> Self {
        Self {
            identity: StockIdentity {
                symbol,
                exchange: None,
            },
            candles,
            time_provider,
            trade_port,
        }
    }

    /// 根据给定的时间点查找最近的 K 线索引（time <= target 的最后一根）
    fn find_candle_at(&self, target: DateTime<Utc>) -> Option<usize> {
        // candles 按时间升序，找 time <= target 的最后一根
        let mut result = None;
        for (i, c) in self.candles.iter().enumerate() {
            if c.time <= target {
                result = Some(i);
            } else {
                break;
            }
        }
        result
    }
}

#[async_trait]
impl Stock for BacktestStock {
    fn identity(&self) -> &StockIdentity {
        &self.identity
    }

    /// 返回当前逻辑时间对应的 K 线收盘价。
    ///
    /// 通过 `time_provider.now()` 查找时间 <= 当前逻辑时间的最后一根 K 线的收盘价。
    /// 这保证 `TradeService.submit_order()` 中获取的 `latest_price` 是回测环境下的正确价格。
    fn current_price(&self) -> Option<Decimal> {
        let now = self.time_provider.now();
        self.find_candle_at(now).map(|idx| self.candles[idx].close)
    }

    fn latest_candle(&self, _timeframe: TimeFrame) -> Option<Candle> {
        let now = self.time_provider.now();
        self.find_candle_at(now).map(|idx| self.candles[idx].clone())
    }

    fn last_closed_candle(&self, _timeframe: TimeFrame) -> Option<Candle> {
        let now = self.time_provider.now();
        self.find_candle_at(now).and_then(|idx| {
            if idx > 0 {
                Some(self.candles[idx - 1].clone())
            } else {
                None
            }
        })
    }

    fn status(&self) -> StockStatus {
        StockStatus::Online
    }

    /// 订阅 K 线流。
    ///
    /// # 回测核心逻辑
    /// 每次 yield 一根 K 线前执行:
    /// 1. 推进 `FakeClockProvider` 到当前 K 线的时间
    /// 2. 调用 `BacktestTradePort.tick()` 驱动限价单撮合
    /// 3. yield 本根 K 线给引擎
    ///
    /// 引擎的 `run_strategy` 循环照常消费这个 stream，
    /// 完全不知道每根 K 线之间有时钟推进和撮合逻辑在执行。
    fn subscribe(&self, _timeframe: TimeFrame) -> CandleStream {
        let candles = self.candles.clone();
        let tp = self.time_provider.clone();
        let trade = self.trade_port.clone();
        let symbol = self.identity.symbol.clone();

        Box::pin(async_stream::stream! {
            for (i, candle) in candles.iter().enumerate() {
                // 1. 推进时钟到当前 K 线时间
                tp.set_time(candle.time);

                // 2. 驱动限价单撮合
                if let Err(e) = trade.tick(&symbol, candle).await {
                    tracing::warn!("BacktestStock tick error at candle {}: {}", i, e);
                }

                debug!("BacktestStock [{}] yielding candle {} at {}", symbol, i, candle.time);

                // 3. yield K 线给引擎
                yield candle.clone();
            }
        })
    }

    /// 获取历史 K 线数据。
    ///
    /// # 回测约束
    /// 严格按 `start` 和 `end` 参数截断数据，**禁止返回逻辑时间之后的 K 线**，
    /// 防止策略通过 `host.fetchHistory()` 获取未来数据导致回测结果失真。
    async fn fetch_history(
        &self,
        _timeframe: TimeFrame,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Candle>, MarketError> {
        let filtered: Vec<Candle> = self
            .candles
            .iter()
            .filter(|c| c.time >= start && c.time <= end)
            .cloned()
            .collect();
        Ok(filtered)
    }
}

/// # Summary
/// 回测专用 Market 领域服务。
///
/// 实现 `Market` trait，返回包含完整回测逻辑的 `BacktestStock` 聚合根。
/// 所有回测 symbol 映射到同一个预加载的 BacktestStock。
pub struct BacktestMarket {
    stock: Arc<BacktestStock>,
}

impl BacktestMarket {
    /// 创建回测市场实例
    ///
    /// # Arguments
    /// * `symbol` - 回测的证券代码
    /// * `candles` - 历史 K 线数据（按时间升序）
    /// * `time_provider` - 回测时钟
    /// * `trade_port` - 撮合端口
    pub fn new(
        symbol: String,
        candles: Vec<Candle>,
        time_provider: Arc<FakeClockProvider>,
        trade_port: Arc<dyn BacktestTradePort>,
    ) -> Self {
        Self {
            stock: Arc::new(BacktestStock::new(
                symbol,
                candles,
                time_provider,
                trade_port,
            )),
        }
    }
}

#[async_trait]
impl Market for BacktestMarket {
    async fn get_stock(&self, _symbol: &str) -> Result<Arc<dyn Stock>, MarketError> {
        Ok(self.stock.clone())
    }

    async fn search_symbols(
        &self,
        _query: &str,
    ) -> Result<Vec<okane_core::store::port::StockMetadata>, MarketError> {
        Ok(vec![])
    }
}
