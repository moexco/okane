use crate::buffer::RollingBuffer;
use async_trait::async_trait;
use okane_cache::mem::MemCache;
use okane_core::cache::port::CacheExt;
use okane_core::common::{Stock as StockIdentity, TimeFrame};
use okane_core::market::entity::Candle;
use okane_core::market::error::MarketError;
use okane_core::market::port::{CandleStream, MarketDataProvider, Stock, StockStatus};
use okane_core::store::port::MarketStore;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::{broadcast, mpsc};
use tracing::info;

/// # Summary
/// Stock 聚合根的具体实现结构。
///
/// # Invariants
/// - 内部状态完全托管于独占的 MemCache 实例中。
/// - 只有广播通道句柄维护在内存 Mutex 中。
pub(crate) struct StockInner {
    // 证券身份信息
    identity: StockIdentity,
    // 广播通道映射（无法序列化，保留在内存中）
    channels: Mutex<HashMap<TimeFrame, broadcast::Sender<Candle>>>,
    // 独占内存缓存实例
    cache: MemCache,
    // 持久化存储驱动
    store: Arc<dyn MarketStore>,
    // 用于通知清理注册表的通道
    cleanup_tx: mpsc::Sender<String>,
    // 数据源驱动
    provider: Arc<dyn MarketDataProvider>,
}

impl StockInner {
    /// # Summary
    /// 创建并初始化聚合根。
    ///
    /// # Logic
    /// 1. 构造 StockInner 实例并注入独占 Cache。
    /// 2. 启动后台抓取协程。
    ///
    /// # Arguments
    /// * `identity`: 证券身份。
    /// * `cleanup_tx`: 清理通道。
    /// * `provider`: 数据源驱动。
    /// * `cache`: 独占缓存实例。
    /// * `store`: 全局存储驱动。
    ///
    /// # Returns
    /// 返回聚合根实例的强引用 Arc。
    pub fn create(
        identity: StockIdentity,
        cleanup_tx: mpsc::Sender<String>,
        provider: Arc<dyn MarketDataProvider>,
        cache: MemCache,
        store: Arc<dyn MarketStore>,
    ) -> Arc<Self> {
        let stock = Arc::new(Self {
            identity: identity.clone(),
            channels: Mutex::new(HashMap::new()),
            cache,
            store,
            cleanup_tx,
            provider: provider.clone(),
        });

        let fetcher = StockFetcher::new(identity, Arc::downgrade(&stock), provider);
        tokio::spawn(fetcher.run());

        stock
    }

    /// # Summary
    /// 获取行情缓冲区的缓存键。
    ///
    /// # Logic
    /// 根据 TimeFrame 返回静态字符串。
    ///
    /// # Arguments
    /// * `tf`: 周期。
    ///
    /// # Returns
    /// 缓存 Key 字符串。
    fn k_key(tf: TimeFrame) -> &'static str {
        match tf {
            TimeFrame::Minute1 => "k:1m",
            TimeFrame::Minute5 => "k:5m",
            TimeFrame::Hour1 => "k:1h",
            TimeFrame::Day1 => "k:1d",
        }
    }

    /// # Summary
    /// 获取最新快照的缓存键。
    ///
    /// # Logic
    /// 根据 TimeFrame 返回静态字符串。
    ///
    /// # Arguments
    /// * `tf`: 周期。
    ///
    /// # Returns
    /// 缓存 Key 字符串。
    fn l_key(tf: TimeFrame) -> &'static str {
        match tf {
            TimeFrame::Minute1 => "l:1m",
            TimeFrame::Minute5 => "l:5m",
            TimeFrame::Hour1 => "l:1h",
            TimeFrame::Day1 => "l:1d",
        }
    }

    /// # Summary
    /// 获取最近收盘的缓存键。
    ///
    /// # Logic
    /// 根据 TimeFrame 返回静态字符串。
    ///
    /// # Arguments
    /// * `tf`: 周期。
    ///
    /// # Returns
    /// 缓存 Key 字符串。
    fn lc_key(tf: TimeFrame) -> &'static str {
        match tf {
            TimeFrame::Minute1 => "lc:1m",
            TimeFrame::Minute5 => "lc:5m",
            TimeFrame::Hour1 => "lc:1h",
            TimeFrame::Day1 => "lc:1d",
        }
    }

    /// # Summary
    /// 更新内部状态并触发广播分发。
    ///
    /// # Logic
    /// 1. 更新缓存中的最新价格 ("p")。
    /// 2. 更新缓存中的最新 K 线快照 ("l:{tf}")。
    /// 3. 获取并更新缓存中的 RollingBuffer ("k:{tf}")。
    /// 4. 若收盘，更新缓存 ("lc:{tf}") 并异步落库。
    /// 5. 触发广播。
    ///
    /// # Arguments
    /// * `candle`: 新接收到的行情数据。
    /// * `timeframe`: 目标周期。
    ///
    /// # Returns
    /// 无。
    pub async fn update_and_broadcast(&self, candle: Candle, timeframe: TimeFrame) {
        // 更新价格和快照
        let _ = self.cache.set("p", &candle.close).await;
        let _ = self.cache.set(Self::l_key(timeframe), &candle).await;

        // 更新滚动缓冲区 (k:rollingbuff)
        let key = Self::k_key(timeframe);
        let mut buffer = self
            .cache
            .get::<RollingBuffer<Candle>>(key)
            .await
            .unwrap_or_default()
            .unwrap_or_else(|| RollingBuffer::new(200));
        buffer.push(candle.clone());
        let _ = self.cache.set(key, &buffer).await;

        if candle.is_final {
            let _ = self.cache.set(Self::lc_key(timeframe), &candle).await;
            let store = self.store.clone();
            let id = self.identity.clone();
            let c = candle.clone();
            tokio::spawn(async move {
                let _ = store.save_candles(&id, timeframe, &[c]).await;
            });
        }

        let channels = self.channels.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(tx) = channels.get(&timeframe) {
            let _ = tx.send(candle);
        }
    }
}

impl Drop for StockInner {
    /// # Summary
    /// 析构处理。
    ///
    /// # Logic
    /// 发送清理信号到管理端通道。
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
    /// 获取身份。
    ///
    /// # Logic
    /// 直接返回内部持有的身份实体引用。
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
    /// 获取最新价。
    ///
    /// # Logic
    /// 从缓存读取 "p" 对应的值。
    ///
    /// # Arguments
    /// 无。
    ///
    /// # Returns
    /// 价格选项。
    fn current_price(&self) -> Option<f64> {
        futures::executor::block_on(async { self.cache.get::<f64>("p").await.ok().flatten() })
    }

    /// # Summary
    /// 获取最新 K 线。
    ///
    /// # Logic
    /// 从缓存读取指定周期的快照 Key。
    ///
    /// # Arguments
    /// * `timeframe`: 目标周期。
    ///
    /// # Returns
    /// K 线选项。
    fn latest_candle(&self, timeframe: TimeFrame) -> Option<Candle> {
        futures::executor::block_on(async {
            self.cache
                .get::<Candle>(Self::l_key(timeframe))
                .await
                .ok()
                .flatten()
        })
    }

    /// # Summary
    /// 获取最近收盘 K 线。
    ///
    /// # Logic
    /// 从缓存读取指定周期的 lc Key。
    ///
    /// # Arguments
    /// * `timeframe`: 目标周期。
    ///
    /// # Returns
    /// K 线选项。
    fn last_closed_candle(&self, timeframe: TimeFrame) -> Option<Candle> {
        futures::executor::block_on(async {
            self.cache
                .get::<Candle>(Self::lc_key(timeframe))
                .await
                .ok()
                .flatten()
        })
    }

    /// # Summary
    /// 订阅行情实时流。
    ///
    /// # Logic
    /// 获取或创建多周期广播通道并产出异步流。
    ///
    /// # Arguments
    /// * `timeframe`: 订阅周期。
    ///
    /// # Returns
    /// 异步行情流。
    fn subscribe(&self, timeframe: TimeFrame) -> CandleStream {
        let mut channels = self.channels.lock().unwrap_or_else(|e| e.into_inner());
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
    /// 历史行情回溯。
    ///
    /// # Logic
    /// 优先从本地持久化 Store 加载，若不足则调 Provider。
    /// 根据 `end_at` 确定时间窗口。
    ///
    /// # Arguments
    /// * `timeframe`: 周期。
    /// * `limit`: 回溯数量。
    /// * `end_at`: 截止时间。
    ///
    /// # Returns
    /// 历史数据结果向量。
    async fn fetch_history(
        &self,
        timeframe: TimeFrame,
        start: chrono::DateTime<chrono::Utc>,
        end: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<Candle>, MarketError> {
        // 优先从本地数据库加载
        if let Some(local) = self
            .store
            .load_candles(&self.identity, timeframe, start, end)
            .await
            .ok()
            .filter(|l| !l.is_empty())
        {
            return Ok(local);
        }

        // 本地缺失则拉取远端数据
        self.provider
            .fetch_candles(&self.identity, timeframe, start, end)
            .await
    }

    /// # Summary
    /// 获取运行状态。
    ///
    /// # Logic
    /// 默认返回 Online。
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
/// 抓取任务后台逻辑执行器。
struct StockFetcher {
    identity: StockIdentity,
    inner: Weak<StockInner>,
    provider: Arc<dyn MarketDataProvider>,
}

impl StockFetcher {
    /// # Summary
    /// 构造 Fetcher 实例。
    ///
    /// # Logic
    /// 初始化字段。
    ///
    /// # Arguments
    /// * `identity`: 证券身份。
    /// * `inner`: 聚合根弱引用。
    /// * `provider`: 数据源驱动。
    ///
    /// # Returns
    /// 返回 Fetcher 实例。
    fn new(
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
    /// 启动抓取协程。
    ///
    /// # Logic
    /// 循环订阅原始行情流并分发至聚合根更新。
    ///
    /// # Arguments
    /// 无。
    ///
    /// # Returns
    /// 无。
    async fn run(self) {
        info!("Fetcher for {} started", self.identity.symbol);
        if let Ok(mut stream) = self
            .provider
            .subscribe_candles(&self.identity, TimeFrame::Minute1)
            .await
        {
            while let Some(candle) = futures::StreamExt::next(&mut stream).await {
                if let Some(stock) = self.inner.upgrade() {
                    stock.update_and_broadcast(candle, TimeFrame::Minute1).await;
                } else {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::MarketImpl;
    use chrono::{DateTime, Utc};
    use futures::stream;
    use okane_core::market::port::Market;
    use okane_core::store::error::StoreError;
    use okane_core::store::port::MarketStore;

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
            Ok(Box::pin(stream::unfold(rx, |mut rx| async move {
                rx.recv().await.map(|c| (c, rx))
            })))
        }
    }

    struct MockStore;
    #[async_trait]
    impl MarketStore for MockStore {
        async fn save_candles(
            &self,
            _: &StockIdentity,
            _: TimeFrame,
            _: &[Candle],
        ) -> Result<(), StoreError> {
            Ok(())
        }
        async fn load_candles(
            &self,
            _: &StockIdentity,
            _: TimeFrame,
            _: DateTime<Utc>,
            _: DateTime<Utc>,
        ) -> Result<Vec<Candle>, StoreError> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn test_stock_aggregate_lifecycle() {
        let provider = Arc::new(MockProvider);
        let store = Arc::new(MockStore);
        let market = MarketImpl::new(provider, store);
        let symbol = "TEST";
        {
            let stock = market.get_stock(symbol).await.expect("Should get stock");
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            assert!(stock.current_price().is_some());
            assert_eq!(market.active_count(), 1);
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        assert_eq!(market.active_count(), 0);
    }
}
