use crate::common::{Stock, TimeFrame};
use crate::market::entity::Candle;
use crate::market::error::MarketError;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::Stream;
use std::pin::Pin;

/// # Summary
/// K 线数据流别名，使用动态分发的异步流。
pub type CandleStream = Pin<Box<dyn Stream<Item = Candle> + Send>>;

/// # Summary
/// 市场行情数据提供者接口。
///
/// # Invariants
/// - 必须实现 `Send` 和 `Sync` 以支持跨线程异步调用。
#[async_trait]
pub trait MarketDataProvider: Send + Sync {
    /// # Summary
    /// 获取特定证券在指定时间范围内的 K 线数据。
    ///
    /// # Logic
    /// 1. 验证时间范围合法性。
    /// 2. 构建数据源请求。
    /// 3. 执行网络请求并解析响应数据。
    ///
    /// # Arguments
    /// * `stock`: 目标证券实体。
    /// * `timeframe`: K 线周期。
    /// * `start`: 开始时间。
    /// * `end`: 结束时间。
    ///
    /// # Returns
    /// 成功则返回 K 线列表，失败返回 `MarketError`。
    async fn fetch_candles(
        &self,
        stock: &Stock,
        timeframe: TimeFrame,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Candle>, MarketError>;

    /// # Summary
    /// 订阅实时 K 线流。
    ///
    /// # Logic
    /// 1. 建立长连接或开启内部轮询。
    /// 2. 持续产生最新的 K 线数据并推入流中。
    ///
    /// # Arguments
    /// * `stock`: 证券实体。
    /// * `timeframe`: K 线周期。
    ///
    /// # Returns
    /// 成功返回异步流 `CandleStream`，失败返回 `MarketError`。
    async fn subscribe_candles(
        &self,
        stock: &Stock,
        timeframe: TimeFrame,
    ) -> Result<CandleStream, MarketError>;
}
