use async_trait::async_trait;
use okane_core::common::{Stock as StockIdentity, TimeFrame};
use okane_core::market::entity::Candle;
use okane_core::market::error::MarketError;
use okane_core::market::port::{CandleStream, MarketDataProvider, Stock, StockStatus};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::{broadcast, mpsc};
use tracing::{info, warn};

/// # Summary
/// Stock 聚合根的具体实现结构。
///
/// # Invariants
/// - 内部状态访问必须由 Mutex 保护。
/// - 被销毁时必须主动向管理器发送信号。
pub(crate) struct StockInner {
    // 证券身份信息
    identity: StockIdentity,
    // 多周期广播器映射
    channels: Mutex<HashMap<TimeFrame, broadcast::Sender<Candle>>>,
    // 各周期最新 K 线快照缓存
    latest_candles: Mutex<HashMap<TimeFrame, Candle>>,
    // 各周期最近收盘 K 线缓存
    last_closed_candles: Mutex<HashMap<TimeFrame, Candle>>,
    // 当前成交价
    price: Mutex<Option<f64>>,
    // 用于通知清理注册表的通道
    cleanup_tx: mpsc::Sender<String>,
    // 数据源驱动，供聚合根内部调用
    provider: Arc<dyn MarketDataProvider>,
}

impl StockInner {
    /// # Summary
    /// 创建并初始化聚合根。
    ///
    /// # Logic
    /// 1. 构造 StockInner 实例。
    /// 2. 构造配套的 StockFetcher 并注入聚合根的弱引用。
    /// 3. 启动后台抓取协程。
    ///
    /// # Arguments
    /// * `identity`: 证券身份标识。
    /// * `cleanup_tx`: 管理器提供的清理信号通道。
    /// * `provider`: 行情数据源驱动。
    ///
    /// # Returns
    /// 返回包装为 Arc 的聚合根实例强引用。
    pub fn create(
        identity: StockIdentity,
        cleanup_tx: mpsc::Sender<String>,
        provider: Arc<dyn MarketDataProvider>,
    ) -> Arc<Self> {
        let stock = Arc::new(Self {
            identity: identity.clone(),
            channels: Mutex::new(HashMap::new()),
            latest_candles: Mutex::new(HashMap::new()),
            last_closed_candles: Mutex::new(HashMap::new()),
            price: Mutex::new(None),
            cleanup_tx,
            provider: provider.clone(),
        });

        let fetcher = StockFetcher::new(identity, Arc::downgrade(&stock), provider);
        tokio::spawn(fetcher.run());

        stock
    }

    /// # Summary
    /// 更新内部状态并触发广播分发。
    ///
    /// # Logic
    /// 1. 更新价格快照。
    /// 2. 更新最新 K 线。
    /// 3. 若 K 线标记为已收盘，同步更新闭合 K 线缓存。
    /// 4. 通过对应周期的广播发送端分发数据。
    ///
    /// # Arguments
    /// * `candle`: 新收到的行情数据。
    /// * `timeframe`: 数据所属的周期。
    ///
    /// # Returns
    /// 无。
    pub fn update_and_broadcast(&self, candle: Candle, timeframe: TimeFrame) {
        let mut price = self.price.lock().unwrap();
        *price = Some(candle.close);

        if candle.is_final {
            self.last_closed_candles
                .lock()
                .unwrap()
                .insert(timeframe, candle.clone());
        }
        self.latest_candles
            .lock()
            .unwrap()
            .insert(timeframe, candle.clone());

        let channels = self.channels.lock().unwrap();
        if let Some(tx) = channels.get(&timeframe) {
            let _ = tx.send(candle);
        }
    }
}

impl Drop for StockInner {
    /// # Summary
    /// 处理聚合根销毁。
    ///
    /// # Logic
    /// 1. 将自身的 Symbol 发送到清理通道。
    ///
    /// # Arguments
    /// 无。
    ///
    /// # Returns
    /// 无。
    fn drop(&mut self) {
        let _ = self.cleanup_tx.try_send(self.identity.symbol.clone());
    }
}

#[async_trait]
impl Stock for StockInner {
    /// # Summary
    /// 获取身份实体引用。
    ///
    /// # Logic
    /// 1. 返回内部 identity。
    ///
    /// # Arguments
    /// 无。
    ///
    /// # Returns
    /// 证券身份标识引用。
    fn identity(&self) -> &StockIdentity {
        &self.identity
    }

    /// # Summary
    /// 获取即时成交价。
    ///
    /// # Logic
    /// 1. 加锁读取瞬时价格缓存。
    ///
    /// # Arguments
    /// 无。
    ///
    /// # Returns
    /// 价格选项。
    fn current_price(&self) -> Option<f64> {
        *self.price.lock().unwrap()
    }

    /// # Summary
    /// 获取指定周期最新 K 线。
    ///
    /// # Logic
    /// 1. 加锁从最新 K 线缓存中读取。
    ///
    /// # Arguments
    /// * `timeframe`: 目标周期。
    ///
    /// # Returns
    /// K 线选项。
    fn latest_candle(&self, timeframe: TimeFrame) -> Option<Candle> {
        self.latest_candles.lock().unwrap().get(&timeframe).cloned()
    }

    /// # Summary
    /// 获取指定周期最近闭合 K 线。
    ///
    /// # Logic
    /// 1. 加锁从收盘 K 线缓存中读取。
    ///
    /// # Arguments
    /// * `timeframe`: 目标周期。
    ///
    /// # Returns
    /// K 线选项数据。
    fn last_closed_candle(&self, timeframe: TimeFrame) -> Option<Candle> {
        self.last_closed_candles
            .lock()
            .unwrap()
            .get(&timeframe)
            .cloned()
    }

    /// # Summary
    /// 订阅实时流。
    ///
    /// # Logic
    /// 1. 获取或创建特定 TimeFrame 的 broadcast 通道。
    /// 2. 使用 async_stream 转换 Receiver 为异步流。
    ///
    /// # Arguments
    /// * `timeframe`: 目标周期。
    ///
    /// # Returns
    /// 动态分发的 Candle 流。
    fn subscribe(&self, timeframe: TimeFrame) -> CandleStream {
        let mut channels = self.channels.lock().unwrap();
        let tx = channels.entry(timeframe).or_insert_with(|| {
            let (tx, _) = broadcast::channel(128);
            tx
        });

        let rx = tx.subscribe();
        let stream = async_stream::stream! {
            let mut rx = rx;
            while let Ok(candle) = rx.recv().await {
                yield candle;
            }
        };

        Box::pin(stream)
    }

    /// # Summary
    /// 回溯历史。
    ///
    /// # Logic
    /// 1. 计算时间跨度。
    /// 2. 委托底层 Provider 进行查询。
    ///
    /// # Arguments
    /// * `timeframe`: 周期。
    /// * `limit`: 回溯根数。
    ///
    /// # Returns
    /// 历史 K 线向量。
    async fn fetch_history(
        &self,
        timeframe: TimeFrame,
        limit: usize,
    ) -> Result<Vec<Candle>, MarketError> {
        let now = chrono::Utc::now();
        let start = now - chrono::Duration::days(limit as i64);
        self.provider
            .fetch_candles(&self.identity, timeframe, start, now)
            .await
    }

    /// # Summary
    /// 查看聚合根状态。
    ///
    /// # Logic
    /// 1. 暂硬编码为 Online。
    ///
    /// # Arguments
    /// 无。
    ///
    /// # Returns
    /// 运行状态枚举。
    fn status(&self) -> StockStatus {
        StockStatus::Online
    }
}

/// # Summary
/// 抓取任务后台逻辑。
///
/// # Invariants
/// - 只持有对聚合根的弱引用，确保不阻碍聚合根的销毁。
struct StockFetcher {
    // 标的身份
    identity: StockIdentity,
    // 对聚合根实现 StockInner 的弱引用
    inner: Weak<StockInner>,
    // 数据源
    provider: Arc<dyn MarketDataProvider>,
}

impl StockFetcher {
    /// # Summary
    /// 构造函数。
    ///
    /// # Logic
    /// 1. 结构化赋值成员变量。
    ///
    /// # Arguments
    /// * `identity`: 证券身份。
    /// * `inner`: 聚合根弱引用。
    /// * `provider`: 数据源驱动。
    ///
    /// # Returns
    /// 返回 StockFetcher 实例。
    pub fn new(
        identity: StockIdentity,
        inner: Weak<StockInner>,
        provider: Arc<dyn MarketDataProvider>,
    ) -> Self {
        Self {
            identity,
            inner,
            provider,
        }
    }

    /// # Summary
    /// 协程执行逻辑。
    ///
    /// # Logic
    /// 1. 订阅底层 Provider 的流。
    /// 2. 每当收到数据时，尝试 upgrade。若失败则意味着外部引用已全部失效，任务退出。
    ///
    /// # Arguments
    /// 无。
    ///
    /// # Returns
    /// 无返回值。
    pub async fn run(self) {
        info!("Fetcher for {} started", self.identity.symbol);

        if let Ok(mut stream) = self
            .provider
            .subscribe_candles(&self.identity, TimeFrame::Minute1)
            .await
        {
            while let Some(candle) = futures::StreamExt::next(&mut stream).await {
                if let Some(stock) = self.inner.upgrade() {
                    stock.update_and_broadcast(candle, TimeFrame::Minute1);
                } else {
                    info!(
                        "No active references for {}, fetcher exiting",
                        self.identity.symbol
                    );
                    break;
                }
            }
        } else {
            warn!("Failed to subscribe for {}", self.identity.symbol);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::MarketImpl;
    use chrono::Utc;
    use futures::stream;
    use okane_core::market::port::Market;

    struct MockProvider;
    #[async_trait]
    impl MarketDataProvider for MockProvider {
        async fn fetch_candles(
            &self,
            _: &StockIdentity,
            _: TimeFrame,
            _: chrono::DateTime<Utc>,
            _: chrono::DateTime<Utc>,
        ) -> Result<Vec<Candle>, MarketError> {
            Ok(vec![])
        }
        async fn subscribe_candles(
            &self,
            _: &StockIdentity,
            _: TimeFrame,
        ) -> Result<CandleStream, MarketError> {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            tx.send(Candle {
                time: Utc::now(),
                open: 1.0,
                high: 2.0,
                low: 0.5,
                close: 1.5,
                adj_close: None,
                volume: 100.0,
                is_final: true,
            })
            .ok();

            let s = stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|c| (c, rx)) });
            Ok(Box::pin(s))
        }
    }

    #[tokio::test]
    async fn test_stock_aggregate_lifecycle() {
        let provider = Arc::new(MockProvider);
        let market = MarketImpl::new(provider);
        let symbol = "TEST";

        {
            let stock = market.get_stock(symbol).await.expect("Should get stock");
            assert_eq!(stock.identity().symbol, symbol);

            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            assert!(stock.current_price().is_some());
            assert_eq!(market.active_count(), 1);
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        assert_eq!(market.active_count(), 0);
    }
}
