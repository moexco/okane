use std::sync::Arc;

use okane_engine::factory::EngineFactory;
use okane_feed::yahoo::YahooProvider;
use okane_manager::strategy::StrategyManager;
use okane_market::manager::MarketImpl;
use okane_store::market::SqliteMarketStore;
use okane_store::strategy::SqliteStrategyStore;
use tracing::info;

/// # Summary
/// 应用启动入口，纯粹的 DI 容器。
/// 负责实例化所有具体实现组件并通过 Arc<dyn Trait> 注入到 StrategyManager。
///
/// # Logic
/// 1. 初始化全局日志。
/// 2. 实例化基础设施层（Feed、Store）。
/// 3. 实例化领域实现层（Market、Engine）。
/// 4. 构造应用服务层（StrategyManager）。
/// 5. 挂起等待外部信号退出。
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. 初始化日志
    tracing_subscriber::fmt::init();
    info!("Okane Engine starting...");

    // 2. 实例化基础设施层
    let feed = Arc::new(YahooProvider::new());
    let market_store = Arc::new(SqliteMarketStore::new()?);
    let strategy_store = Arc::new(SqliteStrategyStore::new()?);

    // 3. 实例化领域实现层
    let market = MarketImpl::new(feed, market_store);

    // 4. 实例化引擎工厂（App 层知道具体实现，Manager 不知道）
    let engine_builder = Arc::new(EngineFactory::new(market.clone()));

    // 5. 构造应用服务层（注入 Core Trait 抽象）
    let _manager = StrategyManager::new(strategy_store, engine_builder);

    info!("StrategyManager initialized. Waiting for signals...");

    // 6. 挂起主线程，等待外部退出信号
    tokio::signal::ctrl_c().await?;
    info!("Shutdown signal received. Exiting...");

    Ok(())
}
