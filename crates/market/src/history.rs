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
use crate::buffer::RollingBuffer;
use std::sync::Mutex;
use tracing::debug;

/// # Summary
/// 回测专用的 Stock 聚合根。
///
/// # Invariants
/// - 内部使用 RollingBuffer 维护最近的 K 线窗口，避免全量载入导致的 OOM。
/// - `subscribe()` 返回的 stream 在每次 yield K 线前自动推进时钟和驱动撮合。
/// - `current_price()` 等方法基于缓存窗口获取。
pub struct BacktestStock {
    identity: StockIdentity,
    /// 固定容量的活跃 K 线窗口（默认 1000 根），用于满足 strategy 的 fetch_history 请求
    buffer: Arc<Mutex<RollingBuffer<Candle>>>,
    /// 回测起点
    start_time: DateTime<Utc>,
    /// 回测终点
    end_time: DateTime<Utc>,
    /// 原始数据源（用于流式补货）
    source: Option<Arc<dyn Stock>>,
    /// 静态数据源（兼容原有 Vec 模式）
    static_candles: Option<Vec<Candle>>,
    /// 回测时钟
    time_provider: Arc<FakeClockProvider>,
    /// 撮合端口
    trade_port: Arc<dyn BacktestTradePort>,
}

impl BacktestStock {
    /// 创建回测用 Stock 聚合根（流式模式）
    pub fn with_source(
        symbol: String,
        source: Arc<dyn Stock>,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        time_provider: Arc<FakeClockProvider>,
        trade_port: Arc<dyn BacktestTradePort>,
    ) -> Self {
        Self {
            identity: StockIdentity { symbol, exchange: None },
            buffer: Arc::new(Mutex::new(RollingBuffer::new(1000))),
            start_time: start,
            end_time: end,
            source: Some(source),
            static_candles: None,
            time_provider,
            trade_port,
        }
    }

    /// 兼容旧有的 Vec 模式
    pub fn new(
        symbol: String,
        candles: Vec<Candle>,
        time_provider: Arc<FakeClockProvider>,
        trade_port: Arc<dyn BacktestTradePort>,
    ) -> Self {
        let (start, end) = if let (Some(f), Some(l)) = (candles.first(), candles.last()) {
            (f.time, l.time)
        } else {
            (Utc::now(), Utc::now())
        };
        Self {
            identity: StockIdentity { symbol, exchange: None },
            buffer: Arc::new(Mutex::new(RollingBuffer::new(candles.len().max(1)))),
            start_time: start,
            end_time: end,
            source: None,
            static_candles: Some(candles),
            time_provider,
            trade_port,
        }
    }
}

#[async_trait]
impl Stock for BacktestStock {
    fn identity(&self) -> &StockIdentity {
        &self.identity
    }

    fn current_price(&self) -> Result<Option<Decimal>, MarketError> {
        let buffer = self.buffer.lock().map_err(|e| MarketError::Unknown(e.to_string()))?;
        Ok(buffer.last().map(|c| c.close))
    }

    fn latest_candle(&self, _timeframe: TimeFrame) -> Result<Option<Candle>, MarketError> {
        let buffer = self.buffer.lock().map_err(|e| MarketError::Unknown(e.to_string()))?;
        Ok(buffer.last())
    }

    fn last_closed_candle(&self, _timeframe: TimeFrame) -> Result<Option<Candle>, MarketError> {
        let buffer = self.buffer.lock().map_err(|e| MarketError::Unknown(e.to_string()))?;
        let vec = buffer.to_vec();
        if vec.len() >= 2 {
            Ok(Some(vec[vec.len() - 2].clone()))
        } else {
            Ok(None)
        }
    }

    fn status(&self) -> StockStatus {
        StockStatus::Online
    }

    fn subscribe(&self, timeframe: TimeFrame) -> Result<CandleStream, MarketError> {
        let tp = self.time_provider.clone();
        let trade = self.trade_port.clone();
        let symbol = self.identity.symbol.clone();
        let buffer = self.buffer.clone();
        
        let source = self.source.clone();
        let static_candles = self.static_candles.clone();
        let start = self.start_time;
        let end = self.end_time;

        Ok(Box::pin(async_stream::stream! {
            let mut current = start;
            while current < end {
                let candles = if let Some(ref s) = source {
                    // 批量拉取数据，提高效率
                    let limit = 100u32;
                    let duration = timeframe.duration() * (i32::try_from(limit).unwrap_or(0));
                    match s.fetch_history(timeframe, current, current + duration).await {
                        Ok(h) => h,
                        Err(e) => {
                            debug!("Streaming fetch failed for {}: {}", symbol, e);
                            break;
                        }
                    }
                } else if let Some(ref sc) = static_candles {
                    sc.iter()
                        .filter(|c| c.time >= current && c.time < end)
                        .take(100)
                        .cloned()
                        .collect::<Vec<_>>()
                } else {
                    break;
                };

                if candles.is_empty() {
                    break;
                }

                for candle in candles {
                    if candle.time > end {
                        break;
                    }
                    // 推进虚拟时钟
                    if let Err(e) = tp.set_time(candle.time) {
                        debug!("Clock set failed: {}", e);
                    }
                    
                    // 更新缓冲区
                    {
                        if let Ok(mut b) = buffer.lock() {
                            b.push(candle.clone());
                        }
                    }

                    // 驱动撮合 (非致命，记录日志并忽略)
                    trade.tick(&symbol, &candle).await.ok();

                    let candle_time = candle.time;
                    yield candle;
                    current = candle_time + timeframe.duration();
                }
            }
        }))
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
        let now = self.time_provider.now().map_err(|e| MarketError::Unknown(e.to_string()))?;
        
        // 核心加固：禁止获取当前回测时刻之后的数据
        let safe_end = std::cmp::min(end, now);

        let buffer = self.buffer.lock().map_err(|e| MarketError::Unknown(e.to_string()))?;
        let filtered: Vec<Candle> = buffer.to_vec()
            .into_iter()
            .filter(|c| c.time >= start && c.time <= safe_end)
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
    /// 创建回测市场实例 (流式)
    pub fn with_source(
        symbol: String,
        source_stock: Arc<dyn Stock>,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        time_provider: Arc<FakeClockProvider>,
        trade_port: Arc<dyn BacktestTradePort>,
    ) -> Self {
        Self {
            stock: Arc::new(BacktestStock::with_source(
                symbol,
                source_stock,
                start,
                end,
                time_provider,
                trade_port,
            )),
        }
    }

    /// 创建回测市场实例 (兼容 Vec)
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
