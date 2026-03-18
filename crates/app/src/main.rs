mod backtest;

use std::sync::Arc;

use okane_core::common::RealTimeProvider;
use okane_engine::factory::EngineFactory;
use okane_feed::yahoo::YahooProvider;
use okane_manager::strategy::StrategyManager;
use okane_market::indicator::MarketIndicatorService;
use okane_market::manager::MarketImpl;
use okane_store::market::SqliteMarketStore;
use okane_store::strategy::SqliteStrategyStore;
use okane_store::system::SqliteSystemStore;
use okane_trade::algo::AlgoOrderService;
use okane_trade::service::TradeService;
use tracing::info;
use tracing_subscriber::fmt::writer::MakeWriterExt;

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
    // 1. 初始化两路输出日志 (控制台 + 滚动文件)
    let file_appender = tracing_appender::rolling::daily("logs", "okane-engine.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .with_writer(
            std::io::stdout
                .with_max_level(tracing::Level::INFO)
                .and(non_blocking.with_max_level(tracing::Level::DEBUG)),
        )
        .with_ansi(true) // 保持控制台颜色，如果嫌麻烦可以拆分 layer
        .init();

    info!("Okane Engine starting...");

    // 1.5 加载全局配置
    let config_file_path = std::path::Path::new("config.toml");
    let mut builder = config::Config::builder();

    if config_file_path.exists() {
        builder = builder.add_source(config::File::from(config_file_path).required(true));
    } else if std::path::Path::new("config").exists() {
        // Support legacy no-extension 'config' file
        builder = builder.add_source(config::File::with_name("config").required(true));
    }

    builder = builder.add_source(config::Environment::with_prefix("OKANE").separator("_"));

    let config_val = builder.build()?;

    // 如果配置了解析失败应当报错退出，否则在完全无配置的情况下回退 Default
    let app_config: okane_core::config::AppConfig = if config_file_path.exists()
        || std::path::Path::new("config").exists()
        || std::env::var("OKANE_SERVER_PORT").is_ok()
    {
        config_val.try_deserialize()?
    } else {
        okane_core::config::AppConfig::default()
    };

    info!("Configuration loaded: {:?}", app_config);

    // 设置全局数据目录 (为 Store 层提供根路径)
    okane_store::config::set_root_dir(std::path::PathBuf::from(
        app_config.database.data_dir.clone(),
    ));

    // 2. 实例化基础设施层
    let feed = Arc::new(YahooProvider::new()?);
    let market_store = Arc::new(SqliteMarketStore::new()?);
    let strategy_store = Arc::new(SqliteStrategyStore::new()?);

    // 3. 实例化领域实现层
    let market = MarketImpl::new(feed, market_store);

    // 4. 实例化引擎工厂（App 层知道具体实现，Manager 不知道）
    let engine_builder = Arc::new(EngineFactory::new(market.clone()));

    // 5. 实例化交易、算法单与指标服务
    let account_store = Arc::new(okane_store::account::SqliteAccountStore::new()?);
    let pending_port = Arc::new(okane_store::pending_order_sqlx::SqlitePendingOrderStore::new()?);
    let matcher = Arc::new(okane_trade::matcher::LocalMatchEngine::new(
        rust_decimal::Decimal::ZERO,
    ));
    let real_time = Arc::new(RealTimeProvider);

    // 核心解耦方案：利用 TradeService 的可选算法单注入
    // 算法单服务需要 TradePort 以执行子单
    let trade_service_base = Arc::new(TradeService::new(
        account_store.clone(),
        matcher.clone(),
        market.clone(),
        pending_port.clone(),
        real_time.clone(),
    ));

    let algo_port = Arc::new(AlgoOrderService::new(
        trade_service_base.clone(),
        real_time.clone(),
    ));

    // 真正的交易服务入口，注入算法单服务以便驱动其运作
    let trade_service = Arc::new(
        TradeService::new(
            account_store,
            matcher,
            market.clone(),
            pending_port,
            real_time,
        )
        .with_algo_service(algo_port.clone()),
    );

    let indicator_service = Arc::new(MarketIndicatorService::new(market.clone()));

    // 6. 实例化系统级存储（提供给鉴权系统 + 用户通知配置查询）
    let system_store: Arc<dyn okane_core::store::port::SystemStore> =
        Arc::new(SqliteSystemStore::new().await?);

    // 7. 创建通知工厂（根据用户 ID 动态创建 Notifier, 配置存储在数据库中）
    let notifier_factory: Arc<dyn okane_core::notify::port::NotifierFactory> = Arc::new(
        okane_notify::factory::DefaultNotifierFactory::new(system_store.clone()),
    );

    // 8. 构造应用服务层（注入 Core Trait 抽象）
    let manager = StrategyManager::new(okane_manager::strategy::StrategyManagerParams {
        store: strategy_store.clone(),
        engine_builder,
        trade_port: trade_service.clone(),
        algo_port: algo_port.clone(),
        indicator_service: indicator_service.clone(),
        time_provider: Arc::new(RealTimeProvider),
        notifier_factory,
        log_port: strategy_store,
    });

    info!("StrategyManager initialized.");

    // 8. 创建回测引擎
    // 回测运行器需要一个工厂函数，以便为隔离的 BacktestMarket 创建 EngineBuilder
    let engine_builder_factory = Arc::new(|m: Arc<dyn okane_core::market::port::Market>| {
        Arc::new(EngineFactory::new(m)) as Arc<dyn okane_core::engine::port::EngineBuilder>
    });
    let backtest_runner = Arc::new(okane_manager::backtest::BacktestRunner::new(
        market.clone(),
        engine_builder_factory,
        Arc::new(backtest::DefaultBacktestEnvironmentFactory),
    ));

    // 9. 挂载 API 服务
    let session_cache = Arc::new(dashmap::DashMap::new());
    let active_sessions = system_store.list_active_sessions().await?;
    for s in active_sessions {
        session_cache.insert(s.id.clone(), s);
    }
    info!(
        "Loaded {} active sessions into memory.",
        session_cache.len()
    );

    let app_state = okane_api::server::AppState {
        strategy_manager: manager.clone(),
        trade_port: trade_service,
        algo_port,
        indicator_service,
        system_store,
        market_port: market.clone(),
        backtest_runner,
        app_config: Arc::new(app_config.clone()),
        session_cache,
    };

    let bind_addr = format!("{}:{}", app_config.server.host, app_config.server.port);
    let bind_addr_clone = bind_addr.clone();
    tokio::spawn(async move {
        if let Err(e) = okane_api::server::start_server(app_state, &bind_addr_clone).await {
            tracing::error!("API server failed: {}", e);
        }
    });

    // 9. 挂起主线程，等待外部退出信号
    info!("Engine and API gateways are fully running. Waiting for signals...");
    tokio::signal::ctrl_c().await?;
    info!("Shutdown signal received. Exiting...");

    Ok(())
}
