use async_trait::async_trait;
use chrono::{DateTime, Utc};
use okane_core::common::TimeFrame;
use okane_core::common::time::TimeProvider;
use okane_core::engine::port::{EngineBuildParams, EngineBuilder};
use okane_core::market::indicator::IndicatorService;
use okane_core::market::port::{Market, Stock};
use okane_core::strategy::entity::EngineType;
use okane_core::trade::entity::{AccountId, AccountSnapshot, Trade};
use okane_core::trade::port::{AlgoOrderPort, BacktestTradePort};
use rust_decimal::Decimal;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::info;

use crate::strategy::ManagerError;

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
pub type EngineBuilderFactory =
    Arc<dyn Fn(Arc<dyn Market>) -> Arc<dyn EngineBuilder> + Send + Sync>;

/// # Summary
/// 回测结果收集端口。
#[async_trait]
pub trait BacktestResultCollector: Send + Sync {
    /// # Logic
    /// Collect the final account snapshot after the backtest finishes.
    ///
    /// # Arguments
    /// * `account_id` - Backtest account identifier.
    ///
    /// # Returns
    /// * `Result<AccountSnapshot, ManagerError>` - Final account snapshot or an error.
    async fn final_snapshot(&self, account_id: &AccountId)
    -> Result<AccountSnapshot, ManagerError>;

    /// # Logic
    /// Drain and return all recorded trades for the finished backtest.
    ///
    /// # Arguments
    /// None.
    ///
    /// # Returns
    /// * `Result<Vec<Trade>, ManagerError>` - Recorded trades or an error.
    async fn drain_trades(&self) -> Result<Vec<Trade>, ManagerError>;
}

/// # Summary
/// 完整的回测运行时环境。
pub struct BacktestEnvironment {
    pub market: Arc<dyn Market>,
    pub trade_port: Arc<dyn BacktestTradePort>,
    pub algo_port: Arc<dyn AlgoOrderPort>,
    pub indicator_service: Arc<dyn IndicatorService>,
    pub time_provider: Arc<dyn TimeProvider>,
    pub account_id: AccountId,
    pub result_collector: Arc<dyn BacktestResultCollector>,
    pub candle_counter: Arc<AtomicUsize>,
}

/// # Summary
/// 回测环境工厂。
#[async_trait]
pub trait BacktestEnvironmentFactory: Send + Sync {
    /// # Logic
    /// Build an isolated backtest environment for a single request.
    ///
    /// # Arguments
    /// * `req` - Backtest request parameters.
    /// * `source_stock` - Source market stock used to seed the backtest environment.
    ///
    /// # Returns
    /// * `Result<BacktestEnvironment, ManagerError>` - Ready-to-run environment or an error.
    async fn create(
        &self,
        req: &BacktestRequest,
        source_stock: Arc<dyn Stock>,
    ) -> Result<BacktestEnvironment, ManagerError>;
}

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
    /// 回测环境工厂 — 由外层注入具体实现层对象
    environment_factory: Arc<dyn BacktestEnvironmentFactory>,
}

impl BacktestRunner {
    /// # Arguments
    /// * `market` - 实盘市场数据源，用于预拉取历史 K 线
    /// * `engine_builder_factory` - 接受 Market 返回 EngineBuilder 的工厂函数。
    ///   回测时传入 BacktestMarket，使引擎读取回测数据。
    pub fn new(
        market: Arc<dyn Market>,
        engine_builder_factory: EngineBuilderFactory,
        environment_factory: Arc<dyn BacktestEnvironmentFactory>,
    ) -> Self {
        Self {
            market,
            engine_builder_factory,
            environment_factory,
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
            ManagerError::Engine(okane_core::engine::error::EngineError::Plugin(format!(
                "Failed to get stock: {}",
                e
            )))
        })?;

        // 步骤 2: 创建隔离的回测上下文
        let environment = self.environment_factory.create(&req, source_stock).await?;

        // 步骤 4: 创建绑定到 BacktestMarket 的 EngineBuilder 并运行
        let engine_builder = (self.engine_builder_factory)(environment.market.clone());

        let engine_future = engine_builder.build(EngineBuildParams {
            engine_type: req.engine_type,
            symbol: req.symbol.clone(),
            account_id: environment.account_id.0.clone(),
            timeframe: req.timeframe,
            source: req.source,
            trade_port: environment.trade_port.clone(),
            algo_port: environment.algo_port.clone(),
            indicator_service: environment.indicator_service.clone(),
            time_provider: environment.time_provider.clone(),
            notifier: None, // 回测中不推送通知
            logger: None,   // 回测日志暂不持久化到核心日志库
        })?;

        // 等待引擎执行完成（BacktestStock 的 stream 耗尽后自动结束）
        engine_future.await?;

        // 步骤 5: 收集结果
        let final_snapshot = environment
            .result_collector
            .final_snapshot(&environment.account_id)
            .await?;
        let trades = environment.result_collector.drain_trades().await?;

        info!(
            "BacktestRunner: completed. {} trades, final balance: {}",
            trades.len(),
            final_snapshot.available_balance
        );

        Ok(BacktestResult {
            final_snapshot,
            trades,
            candle_count: environment.candle_counter.load(Ordering::Relaxed),
        })
    }
}
