use async_trait::async_trait;
use chrono::{DateTime, Utc};
use okane_core::common::time::FakeClockProvider;
use okane_core::common::TimeFrame;
use okane_core::engine::port::{EngineBuildParams, EngineBuilder};

use okane_core::market::port::{Market, Stock};
use okane_core::strategy::entity::EngineType;
use okane_core::trade::entity::AccountSnapshot;
use okane_core::trade::port::AccountPort;
use okane_market::history::BacktestMarket;
use okane_trade::account::AccountManager;
use okane_trade::service::TradeService;
use okane_trade::trade_log::TradeLog;
use rust_decimal::Decimal;
use std::sync::{Arc, Mutex};
use tracing::info;

use crate::strategy::ManagerError;

// ---------------------------------------------------------------------------
// LazyMarket: 打破 TradeService ↔ BacktestMarket 循环引用
// ---------------------------------------------------------------------------

/// 延迟初始化的 Market 包装器。
///
/// 用于打破 `TradeService` ↔ `BacktestMarket` 的循环引用:
/// - `BacktestMarket` 需要 `Arc<dyn BacktestTradePort>` (= TradeService)
/// - `TradeService` 需要 `Arc<dyn Market>` (= BacktestMarket, 用于 current_price)
///
/// 构造顺序:
/// 1. 创建 `LazyMarket`
/// 2. 用 `LazyMarket` 创建 `TradeService`
/// 3. 用 `TradeService` 创建 `BacktestMarket`
/// 4. 用 `LazyMarket.set()` 注入 `BacktestMarket`
/// # Summary
/// 采用懒加载模式的 Market 代理，用于破解 `TradeService` 与 `Market` 之间的循环依赖。
///
/// # Invariants
/// - **循环依赖解耦**: `TradeService` 需要 `Market` 报价以驱动成交逻辑，而 `BacktestMarket` 在初始化 `BacktestStock` 时需要 `TradeService`。
/// - **初始化顺序**: 在回测启动阶段，先创建 `LazyMarket` 并注入 `TradeService`，随后当 `BacktestMarket` 就绪后再通过 `set_inner` 完成最终绑定。
/// - **时序一致性**: 在 `inner` 被设置前调用任何 `Market` 方法都会由于内部 `None` 而静默或报错（取决于实现），这符合回测引擎初始化阶段的原子性要求。
struct LazyMarket {
    inner: Mutex<Option<Arc<dyn Market>>>,
}

impl LazyMarket {
    fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    fn set(&self, market: Arc<dyn Market>) -> Result<(), okane_core::market::error::MarketError> {
        let mut lock = self.inner.lock().map_err(|e| okane_core::market::error::MarketError::Unknown(e.to_string()))?;
        if lock.is_some() {
            return Err(okane_core::market::error::MarketError::Unknown("LazyMarket: already initialized".to_string()));
        }
        *lock = Some(market);
        Ok(())
    }
}

#[async_trait]
impl Market for LazyMarket {
    async fn get_stock(&self, symbol: &str) -> Result<Arc<dyn Stock>, okane_core::market::error::MarketError> {
        let inner = {
            let lock = self.inner.lock().map_err(|e| okane_core::market::error::MarketError::Unknown(e.to_string()))?;
            lock.as_ref().cloned().ok_or_else(|| okane_core::market::error::MarketError::Unknown("LazyMarket: inner market not initialized".to_string()))?
        };
        inner.get_stock(symbol).await
    }

    async fn search_symbols(
        &self,
        query: &str,
    ) -> Result<Vec<okane_core::store::port::StockMetadata>, okane_core::market::error::MarketError> {
        let inner = {
            let lock = self.inner.lock().map_err(|e| okane_core::market::error::MarketError::Unknown(e.to_string()))?;
            lock.as_ref().cloned().ok_or_else(|| okane_core::market::error::MarketError::Unknown("LazyMarket: inner market not initialized".to_string()))?
        };
        inner.search_symbols(query).await
    }
}

// ---------------------------------------------------------------------------
// BacktestRunner
// ---------------------------------------------------------------------------

/// # Summary
/// 回测请求参数。
pub struct BacktestRequest {
    /// 证券代码
    pub symbol: String,
    /// K 线周期
    pub timeframe: TimeFrame,
    /// 回测开始时间
    pub start: DateTime<Utc>,
    /// 回测结束时间
    pub end: DateTime<Utc>,
    /// 引擎类型 (JavaScript / Wasm)
    pub engine_type: EngineType,
    /// 策略源码 (JS 文本或 WASM 字节码)
    pub source: Vec<u8>,
    /// 初始资金
    pub initial_balance: Decimal,
}

/// # Summary
/// 回测结果。
pub struct BacktestResult {
    /// 最终账户快照
    pub final_snapshot: AccountSnapshot,
    /// 完整的交易历史
    pub trades: Vec<okane_core::trade::entity::Trade>,
    /// 处理的 K 线数量
    pub candle_count: usize,
}

/// 引擎构建器的工厂函数类型
pub type EngineBuilderFactory = Arc<dyn Fn(Arc<dyn Market>) -> Arc<dyn EngineBuilder> + Send + Sync>;

/// # Summary
/// 回测运行器。
///
/// 负责组装完整的回测环境并执行策略：
/// 1. 从实盘 Market 预拉取历史 K 线
/// 2. 创建隔离的回测上下文（BacktestMarket, FakeClockProvider, AccountManager）
/// 3. 通过 EngineBuilder 构建并运行策略
/// 4. 收集结果返回
///
/// # Invariants
/// - 每次回测使用完全隔离的账户和市场数据，互不干扰。
/// - 策略在回测中完全无感知，与实盘运行行为一致。
pub struct BacktestRunner {
    /// 实盘市场数据源 — 用于预拉取历史 K 线
    market: Arc<dyn Market>,
    /// 引擎构建器的工厂函数 — 回测需要注入 BacktestMarket 作为 Market
    engine_builder_factory: EngineBuilderFactory,
}

impl BacktestRunner {
    /// # Arguments
    /// * `market` - 实盘市场数据源，用于预拉取历史 K 线
    /// * `engine_builder_factory` - 接受 Market 返回 EngineBuilder 的工厂函数。
    ///   回测时传入 BacktestMarket，使引擎读取回测数据。
    pub fn new(
        market: Arc<dyn Market>,
        engine_builder_factory: EngineBuilderFactory,
    ) -> Self {
        Self {
            market,
            engine_builder_factory,
        }
    }

    /// 执行一次完整回测。
    ///
    /// # Logic
    /// 1. 从实盘 Market 预拉取 [start, end] 时间窗口内的历史 K 线。
    /// 2. 创建隔离的回测上下文:
    ///    - `FakeClockProvider`: 从 start 开始
    ///    - `AccountManager`: 仅包含测试账户
    ///    - `TradeService`: 注入 FakeClockProvider + TradeLog
    ///    - `BacktestMarket`: 持有历史 K 线 + 时钟 + 撮合
    /// 3. 用 LazyMarket 打破循环引用，延迟注入 BacktestMarket。
    /// 4. 构建并运行引擎任务（stream 耗尽后自动结束）。
    /// 5. 收集 AccountSnapshot + TradeLog 并返回。
    pub async fn run(&self, req: BacktestRequest) -> Result<BacktestResult, ManagerError> {
        info!(
            "BacktestRunner: starting for [{}] from {} to {}, engine={:?}",
            req.symbol, req.start, req.end, req.engine_type
        );

        // 步骤 1: 获取原始 Stock 句柄 (不再全量拉取 fetch_history)
        let source_stock = self.market.get_stock(&req.symbol).await.map_err(|e| {
            ManagerError::Engine(okane_core::engine::error::EngineError::Plugin(
                format!("Failed to get stock: {}", e),
            ))
        })?;

        // 步骤 2: 创建隔离的回测上下文
        let fake_clock = Arc::new(FakeClockProvider::new(req.start));
        let account_manager = Arc::new(AccountManager::new());
        let backtest_account_id =
            okane_core::trade::entity::AccountId(format!("backtest_{}", uuid::Uuid::new_v4()));
        account_manager.ensure_account_exists(backtest_account_id.clone(), req.initial_balance);

        let pending_port = Arc::new(okane_store::pending_order::MemoryPendingOrderStore::new());
        let matcher = Arc::new(okane_trade::matcher::LocalMatchEngine::new(Decimal::ZERO));
        let trade_log = Arc::new(TradeLog::new());

        // 步骤 3: 用 LazyMarket 打破循环引用
        let lazy_market: Arc<LazyMarket> = Arc::new(LazyMarket::new());

        let trade_service = Arc::new(
            TradeService::new(
                account_manager.clone(),
                matcher,
                lazy_market.clone(), // 先传入空壳 Market
                pending_port,
                fake_clock.clone(),
            )
            .with_trade_log(trade_log.clone()),
        );

        // 现在用 trade_service 创建 BacktestMarket (流式模式)
        let backtest_market: Arc<dyn Market> = Arc::new(BacktestMarket::with_source(
            req.symbol.clone(),
            source_stock,
            req.start,
            req.end,
            fake_clock.clone(),
            trade_service.clone(),
        ));

        // 延迟注入: LazyMarket 现在指向 BacktestMarket
        lazy_market.set(backtest_market.clone()).map_err(|e| {
            ManagerError::Engine(okane_core::engine::error::EngineError::Plugin(
                format!("Failed to initialize lazy market: {}", e),
            ))
        })?;

        // 步骤 4: 创建绑定到 BacktestMarket 的 EngineBuilder 并运行
        let engine_builder = (self.engine_builder_factory)(backtest_market);

        let engine_future = engine_builder.build(EngineBuildParams {
            engine_type: req.engine_type,
            symbol: req.symbol.clone(),
            account_id: backtest_account_id.0.clone(),
            timeframe: req.timeframe,
            source: req.source,
            trade_port: trade_service.clone(),
            time_provider: fake_clock,
            notifier: None, // 回测中不推送通知
        })?;

        // 等待引擎执行完成（BacktestStock 的 stream 耗尽后自动结束）
        engine_future.await?;

        // 步骤 5: 收集结果
        let final_snapshot = account_manager.snapshot(&backtest_account_id).await?;
        let trades = trade_log.drain()?;

        info!(
            "BacktestRunner: completed. {} trades, final balance: {}",
            trades.len(),
            final_snapshot.available_balance
        );

        Ok(BacktestResult {
            final_snapshot,
            trades,
            candle_count: 0, // 流式模式下不再预先计数，此处设为 0 或记录实耗数
        })
    }
}
