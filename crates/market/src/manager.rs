use crate::stock::StockInner;
use async_trait::async_trait;
use dashmap::DashMap;
use okane_core::common::Stock as StockIdentity;
use okane_core::market::error::MarketError;
use okane_core::market::port::{Market, MarketDataProvider, Stock};
use std::sync::{Arc, Weak};
use tokio::sync::mpsc;
use tracing::{debug, info};

/// # Summary
/// Market 领域服务的具体实现类。
///
/// # Invariants
/// - 维护 Symbol 到 Stock 聚合根弱引用的映射。
/// - 内部持有清理通道以接收聚合根销毁信号。
pub struct MarketImpl {
    // 原始行情数据源驱动
    provider: Arc<dyn MarketDataProvider>,
    // 活跃聚合根注册表，Key 为 Symbol，Value 为弱引用
    stocks: DashMap<String, Weak<StockInner>>,
    // 用于接收聚合根销毁信号的发送端
    cleanup_tx: mpsc::Sender<String>,
}

impl MarketImpl {
    /// # Summary
    /// 初始化 Market 领域服务。
    ///
    /// # Logic
    /// 1. 创建 mpsc 通道用于资源清理。
    /// 2. 构造 MarketImpl 实例并包装为 Arc。
    /// 3. 启动后台协程监听清理通道，根据接收到的 Symbol 移除注册表条目。
    ///
    /// # Arguments
    /// * `provider`: 满足 MarketDataProvider 接口的数据源驱动。
    ///
    /// # Returns
    /// 返回 MarketImpl 的共享指针。
    pub fn new(provider: Arc<dyn MarketDataProvider>) -> Arc<Self> {
        let (tx, mut rx) = mpsc::channel(100);
        let market = Arc::new(Self {
            provider,
            stocks: DashMap::new(),
            cleanup_tx: tx,
        });

        let market_clone = Arc::downgrade(&market);
        tokio::spawn(async move {
            info!("Market cleanup monitor started");
            while let Some(symbol) = rx.recv().await {
                if let Some(m) = market_clone.upgrade() {
                    debug!("Cleanup monitor: removing stock {}", symbol);
                    m.stocks.remove(&symbol);
                }
            }
        });

        market
    }

    /// # Summary
    /// 获取当前活跃的聚合根数量（仅供测试）。
    ///
    /// # Logic
    /// 1. 返回 stocks Map 的长度。
    ///
    /// # Arguments
    /// 无。
    ///
    /// # Returns
    /// 数量。
    #[cfg(test)]
    pub(crate) fn active_count(&self) -> usize {
        self.stocks.len()
    }
}

#[async_trait]
impl Market for MarketImpl {
    /// # Summary
    /// 根据证券代码获取或创建一个聚合根实例。
    ///
    /// # Logic
    /// 1. 尝试从 stocks 注册表中获取 Weak 引用。
    /// 2. 若 Weak 引用能成功 upgrade，说明聚合根活跃，直接返回其 Arc。
    /// 3. 否则，通过 StockInner::create 构造新实例并启动后台抓取任务。
    /// 4. 将新实例的弱引用存入注册表并返回强引用。
    ///
    /// # Arguments
    /// * `symbol`: 证券唯一代码。
    ///
    /// # Returns
    /// 成功返回 Stock 聚合根（Arc 包装），失败返回 MarketError。
    async fn get_stock(&self, symbol: &str) -> Result<Arc<dyn Stock>, MarketError> {
        if let Some(weak) = self.stocks.get(symbol)
            && let Some(arc) = weak.upgrade()
        {
            return Ok(arc);
        }

        let identity = StockIdentity {
            symbol: symbol.to_string(),
            exchange: None,
        };

        let arc_stock =
            StockInner::create(identity, self.cleanup_tx.clone(), self.provider.clone());

        self.stocks
            .insert(symbol.to_string(), Arc::downgrade(&arc_stock));
        Ok(arc_stock)
    }
}
