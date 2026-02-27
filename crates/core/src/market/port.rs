use crate::common::{Stock as StockIdentity, TimeFrame};
use crate::market::entity::Candle;
use crate::market::error::MarketError;
use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;
use std::sync::Arc;

/// # Summary
/// K 线数据流别名，使用动态分发的异步流。
pub type CandleStream = Pin<Box<dyn Stream<Item = Candle> + Send>>;

/// # Summary
/// Stock 聚合根行为契约。
///
/// # Invariants
/// - 身份标识 (Identity) 必须在聚合根生命周期内保持不变。
/// - 同一个周期 (TimeFrame) 的实时流 must be 单源广播的。
#[async_trait]
pub trait Stock: Send + Sync {
    /// # Summary
    /// 获取该聚合根的唯一身份标识。
    ///
    /// # Logic
    /// 1. 返回聚合根内部持有的静态身份数据。
    ///
    /// # Returns
    /// 返回证券身份实体引用。
    fn identity(&self) -> &StockIdentity;

    /// # Summary
    /// 获取当前最新成交价。
    ///
    /// # Logic
    /// 1. 从内存中的最新状态快照中读取成交价。
    ///
    /// # Returns
    /// 若有价格则返回成交价，否则返回 None。
    fn current_price(&self) -> Option<f64>;

    /// # Summary
    /// 获取特定周期下正在形成中的最新 K 线快照。
    ///
    /// # Logic
    /// 1. 根据 TimeFrame 路由到对应的 K 线聚合器。
    /// 2. 获取当前尚未闭合的 K 线数据。
    ///
    /// # Arguments
    /// * `timeframe`: K 线周期。
    ///
    /// # Returns
    /// 返回当前未收盘的 K 线数据。
    fn latest_candle(&self, timeframe: TimeFrame) -> Option<Candle>;

    /// # Summary
    /// 获取特定周期下刚刚收盘的完整 K 线。
    ///
    /// # Logic
    /// 1. 从内存缓存中读取最近一根已闭合的 K 线。
    ///
    /// # Arguments
    /// * `timeframe`: K 线周期。
    ///
    /// # Returns
    /// 返回最近一根已闭合的 K 线数据。
    fn last_closed_candle(&self, timeframe: TimeFrame) -> Option<Candle>;

    /// # Summary
    /// 订阅该证券的实时行情流。
    ///
    /// # Logic
    /// 1. 挂载到聚合根内部的广播器。
    /// 2. 持续接收并产出最新的行情事件。
    ///
    /// # Arguments
    /// * `timeframe`: 订阅的 K 线周期。
    ///
    /// # Returns
    /// 返回异步流 CandleStream。
    fn subscribe(&self, timeframe: TimeFrame) -> CandleStream;

    /// # Summary
    /// 获取该聚合根关联的历史数据。
    ///
    /// # Logic
    /// 1. 尝试从本地缓存或持久层回溯数据。
    /// 2. 若本地缺失，则向原始提供者请求补全。
    ///
    /// # Arguments
    /// * `timeframe`: K 线周期。
    /// * `limit`: 请求的数量上限。
    /// * `end_at`: 可选的截止时间（包含），若为 None 则表示从最新时刻向前回溯。
    ///
    /// # Returns
    /// 成功返回 K 线列表，失败返回 MarketError。
    async fn fetch_history(
        &self,
        timeframe: TimeFrame,
        limit: usize,
        end_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Vec<Candle>, MarketError>;

    /// # Summary
    /// 获取聚合根当前的运行状态。
    ///
    /// # Logic
    /// 1. 返回聚合根内部任务监控器的状态。
    ///
    /// # Returns
    /// 返回 StockStatus 枚举。
    fn status(&self) -> StockStatus;
}

/// # Summary
/// 聚合根运行状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StockStatus {
    Initializing,
    Online,
    Offline,
    Faulted,
}

/// # Summary
/// 市场行情数据提供者接口（原始数据源）。
///
/// # Invariants
/// - 实现者必须保证 subscribe_candles 在长连接中断后具备自愈能力或通过轮询降级。
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
    /// * `stock`: 证券身份。
    /// * `timeframe`: K 线周期。
    /// * `start`: 开始时间。
    /// * `end`: 结束时间。
    ///
    /// # Returns
    /// 成功返回 K 线列表。
    async fn fetch_candles(
        &self,
        stock: &StockIdentity,
        timeframe: TimeFrame,
        start: chrono::DateTime<chrono::Utc>,
        end: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<Candle>, MarketError>;

    /// # Summary
    /// 订阅实时 K 线流。
    ///
    /// # Logic
    /// 1. 建立长连接或开启内部轮询。
    /// 2. 持续产生最新的 K 线数据并推入流中。
    ///
    /// # Arguments
    /// * `stock`: 证券身份。
    /// * `timeframe`: K 线周期。
    ///
    /// # Returns
    /// 成功返回异步流。
    async fn subscribe_candles(
        &self,
        stock: &StockIdentity,
        timeframe: TimeFrame,
    ) -> Result<CandleStream, MarketError>;
}

/// # Summary
/// Market 领域服务契约（工厂与注册表）。
///
/// # Invariants
/// - 必须维持 Symbol 到物理聚合根的唯一映射。
/// - 负责聚合根在零引用时的资源回收。
#[async_trait]
pub trait Market: Send + Sync {
    /// # Summary
    /// 根据 Symbol 获取或创建一个 Stock 聚合根。
    ///
    /// # Logic
    /// 1. 在活跃注册表中查找对应 Symbol。
    /// 2. 若存在且有效，返回其强引用。
    /// 3. 若不存在，初始化新的聚合根并启动抓取任务，存入注册表。
    ///
    /// # Arguments
    /// * `symbol`: 证券代码。
    ///
    /// # Returns
    /// 成功返回 Stock 聚合根。
    async fn get_stock(&self, symbol: &str) -> Result<Arc<dyn Stock>, MarketError>;
}
