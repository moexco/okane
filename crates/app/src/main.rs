use std::sync::Arc;

use okane_engine::factory::EngineFactory;
use okane_feed::yahoo::YahooProvider;
use okane_manager::strategy::StrategyManager;
use okane_market::manager::MarketImpl;
use okane_store::market::SqliteMarketStore;
use okane_store::strategy::SqliteStrategyStore;
use okane_store::system::SqliteSystemStore;
use okane_trade::account::AccountManager;
use okane_trade::service::TradeService;
use okane_core::trade::port::TradePort;
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
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .with_writer(
            std::io::stdout.with_max_level(tracing::Level::INFO)
            .and(non_blocking.with_max_level(tracing::Level::DEBUG))
        )
        .with_ansi(true) // 保持控制台颜色，如果嫌麻烦可以拆分 layer
        .init();

    info!("Okane Engine starting...");

    // 1.5 加载全局配置
    let config_val = config::Config::builder()
        .add_source(config::File::with_name("config").required(false))
        .add_source(config::Environment::with_prefix("OKANE").separator("_"))
        .build()?;
    let app_config: okane_core::config::AppConfig = config_val.try_deserialize().unwrap_or_default();

    info!("Configuration loaded: {:?}", app_config);

    // 设置全局数据目录 (为 Store 层提供根路径)
    okane_store::config::set_root_dir(std::path::PathBuf::from(app_config.database.data_dir.clone()));

    // 2. 实例化基础设施层
    let feed = Arc::new(YahooProvider::new());
    let market_store = Arc::new(SqliteMarketStore::new()?);
    let strategy_store = Arc::new(SqliteStrategyStore::new()?);

    // 3. 实例化领域实现层
    let market = MarketImpl::new(feed, market_store);

    // 4. 实例化引擎工厂（App 层知道具体实现，Manager 不知道）
    let engine_builder = Arc::new(EngineFactory::new(market.clone()));

    // 5. 实例化交易服务（目前使用本地撮合和内存账户）
    let account_manager = Arc::new(AccountManager::default());
    let matcher = std::sync::Arc::new(okane_trade::matcher::LocalMatchEngine::default());
    let trade_service: Arc<dyn TradePort> = Arc::new(TradeService::new(account_manager, matcher, market.clone()));

    // 6. 构造应用服务层（注入 Core Trait 抽象）
    let manager = StrategyManager::new(
        strategy_store,
        engine_builder,
        trade_service.clone(),
    );

    info!("StrategyManager initialized.");

    // 7. 实例化系统级存储，提供给鉴权系统、配置下发
    let system_store: Arc<dyn okane_core::store::port::SystemStore> =
        Arc::new(SqliteSystemStore::new().await?);

    // 8. 挂载 API 服务
    let app_state = okane_api::server::AppState {
        strategy_manager: manager.clone(),
        trade_port: trade_service,
        system_store,
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
