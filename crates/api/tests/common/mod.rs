use async_trait::async_trait;
use okane_api::server::AppState;
use okane_core::market::port::{Market, Stock};
use okane_core::notify::port::NotifierFactory;
use okane_core::store::port::StockMetadata;
use okane_core::store::port::{MarketStore, SystemStore, User, UserRole};
use okane_core::test_utils::MockMarketDataProvider;
use okane_core::trade::port::TradePort;
use okane_engine::factory::EngineFactory;
use okane_manager::backtest::{
    BacktestEnvironment, BacktestEnvironmentFactory, BacktestRequest, BacktestResultCollector,
};
use okane_manager::strategy::StrategyManager;
use okane_market::history::BacktestMarket;
use okane_market::indicator::MarketIndicatorService;
use okane_market::manager::MarketImpl;
use okane_store::market::SqliteMarketStore;
use okane_store::strategy::SqliteStrategyStore;
use okane_store::system::SqliteSystemStore;
use okane_trade::account::AccountManager;
use okane_trade::algo::AlgoOrderService;
use okane_trade::matcher::LocalMatchEngine;
use okane_trade::service::TradeService;
use okane_trade::trade_log::TradeLog;
use rust_decimal::Decimal;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use tokio::net::TcpListener;

/// 测试用空通知工厂
pub struct NoopNotifierFactory;
#[async_trait::async_trait]
impl NotifierFactory for NoopNotifierFactory {
    async fn create_for_user(
        &self,
        _user_id: &str,
    ) -> Result<
        Option<std::sync::Arc<dyn okane_core::notify::port::Notifier>>,
        okane_core::notify::error::NotifyError,
    > {
        Ok(None)
    }
}

struct TestLazyMarket {
    inner: Mutex<Option<Arc<dyn Market>>>,
}

impl TestLazyMarket {
    fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    fn set(&self, market: Arc<dyn Market>) -> Result<(), okane_core::market::error::MarketError> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|e| okane_core::market::error::MarketError::Unknown(e.to_string()))?;
        *guard = Some(market);
        Ok(())
    }
}

#[async_trait]
impl Market for TestLazyMarket {
    async fn get_stock(
        &self,
        symbol: &str,
    ) -> Result<Arc<dyn Stock>, okane_core::market::error::MarketError> {
        let market = self
            .inner
            .lock()
            .map_err(|e| okane_core::market::error::MarketError::Unknown(e.to_string()))?
            .as_ref()
            .cloned()
            .ok_or_else(|| {
                okane_core::market::error::MarketError::Unknown(
                    "lazy market not initialized".to_string(),
                )
            })?;
        market.get_stock(symbol).await
    }

    async fn search_symbols(
        &self,
        query: &str,
    ) -> Result<Vec<okane_core::store::port::StockMetadata>, okane_core::market::error::MarketError>
    {
        let market = self
            .inner
            .lock()
            .map_err(|e| okane_core::market::error::MarketError::Unknown(e.to_string()))?
            .as_ref()
            .cloned()
            .ok_or_else(|| {
                okane_core::market::error::MarketError::Unknown(
                    "lazy market not initialized".to_string(),
                )
            })?;
        market.search_symbols(query).await
    }
}

struct TestBacktestCollector {
    trade_port: Arc<dyn TradePort>,
    trade_log: Arc<TradeLog>,
}

#[async_trait]
impl BacktestResultCollector for TestBacktestCollector {
    async fn final_snapshot(
        &self,
        account_id: &okane_core::trade::entity::AccountId,
    ) -> Result<okane_core::trade::entity::AccountSnapshot, okane_manager::strategy::ManagerError>
    {
        Ok(self.trade_port.get_account(account_id.clone()).await?)
    }

    async fn drain_trades(
        &self,
    ) -> Result<Vec<okane_core::trade::entity::Trade>, okane_manager::strategy::ManagerError> {
        Ok(self.trade_log.drain()?)
    }
}

struct TestBacktestEnvironmentFactory;

#[async_trait]
impl BacktestEnvironmentFactory for TestBacktestEnvironmentFactory {
    async fn create(
        &self,
        req: &BacktestRequest,
        source_stock: Arc<dyn Stock>,
    ) -> Result<BacktestEnvironment, okane_manager::strategy::ManagerError> {
        let fake_clock = Arc::new(okane_core::common::time::FakeClockProvider::new(req.start));
        let account_manager = Arc::new(AccountManager::new());
        let backtest_account_id =
            okane_core::trade::entity::AccountId(format!("backtest_{}", uuid::Uuid::new_v4()));
        account_manager.ensure_account_exists(backtest_account_id.clone(), req.initial_balance);

        let pending_port = Arc::new(okane_store::pending_order::MemoryPendingOrderStore::new());
        let matcher = Arc::new(LocalMatchEngine::new(Decimal::ZERO));
        let trade_log = Arc::new(TradeLog::new());
        let lazy_market = Arc::new(TestLazyMarket::new());
        let candle_counter = Arc::new(AtomicUsize::new(0));

        let trade_service = Arc::new(
            TradeService::new(
                account_manager.clone(),
                matcher,
                lazy_market.clone(),
                pending_port,
                fake_clock.clone(),
            )
            .with_trade_log(trade_log.clone()),
        );

        let backtest_market: Arc<dyn Market> = Arc::new(BacktestMarket::with_source(
            req.symbol.clone(),
            source_stock,
            req.start,
            req.end,
            fake_clock.clone(),
            trade_service.clone(),
            candle_counter.clone(),
        ));
        lazy_market.set(backtest_market.clone()).map_err(|e| {
            okane_manager::strategy::ManagerError::Engine(
                okane_core::engine::error::EngineError::Plugin(format!(
                    "failed to initialize lazy market: {}",
                    e
                )),
            )
        })?;

        let algo_service = Arc::new(AlgoOrderService::new(
            trade_service.clone(),
            fake_clock.clone(),
        ));
        trade_service.set_algo_service(algo_service.clone())?;

        Ok(BacktestEnvironment {
            market: backtest_market.clone(),
            trade_port: trade_service.clone(),
            algo_port: algo_service,
            indicator_service: Arc::new(MarketIndicatorService::new(backtest_market)),
            time_provider: fake_clock,
            account_id: backtest_account_id,
            result_collector: Arc::new(TestBacktestCollector {
                trade_port: trade_service.clone(),
                trade_log,
            }),
            candle_counter,
        })
    }
}

// 帮助函数：在随端口启动测试服务器
pub async fn spawn_test_server() -> anyhow::Result<(
    String,
    Arc<dyn SystemStore>,
    tempfile::TempDir,
    Vec<Arc<dyn Stock>>,
    Arc<MockMarketDataProvider>,
)> {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        tracing_subscriber::fmt()
            .with_env_filter("off")
            .try_init()
            .ok();
    });
    let tmp_dir =
        tempfile::tempdir().map_err(|e| anyhow::anyhow!("Failed to create temp dir: {}", e))?;
    let root_path = tmp_dir.path().to_path_buf();

    let system_store: Arc<dyn SystemStore> = Arc::new(
        SqliteSystemStore::new_with_path(Some(root_path.clone()))
            .await
            .map_err(|e| anyhow::anyhow!("Failed to init SqliteSystemStore: {}", e))?,
    );

    // 强制覆盖 admin 的密码为已知测试密码 "test_admin_pwd"
    let hashed = bcrypt::hash("test_admin_pwd", bcrypt::DEFAULT_COST)
        .map_err(|e| anyhow::anyhow!("Failed to hash admin password: {}", e))?;
    let admin_user = User {
        id: "admin".to_string(),
        name: "Admin".to_string(),
        password_hash: hashed,
        role: UserRole::Admin,
        force_password_change: false,
        created_at: chrono::Utc::now(),
    };
    system_store
        .save_user(&admin_user)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to save admin user: {}", e))?;
    system_store
        .bind_account("admin", "trader_01")
        .await
        .map_err(|e| anyhow::anyhow!("Failed to bind trader_01: {}", e))?;

    let feed = Arc::new(MockMarketDataProvider::new());
    let metadata = StockMetadata {
        symbol: "AAPL".to_string(),
        name: "Apple Inc.".to_string(),
        exchange: "NASDAQ".to_string(),
        sector: Some("Technology".to_string()),
        currency: "USD".to_string(),
    };
    feed.set_search_results(vec![metadata.clone()])?;
    system_store
        .save_stock_metadata(&metadata)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to save stock metadata: {}", e))?;

    let history_time = chrono::Utc::now() - chrono::Duration::hours(1);
    let initial_candle = okane_core::market::entity::Candle {
        time: history_time,
        open: Decimal::from_str("150.00").map_err(|e| anyhow::anyhow!(e))?,
        high: Decimal::from_str("155.00").map_err(|e| anyhow::anyhow!(e))?,
        low: Decimal::from_str("145.00").map_err(|e| anyhow::anyhow!(e))?,
        close: Decimal::from_str("150.00").map_err(|e| anyhow::anyhow!(e))?,
        adj_close: Some(Decimal::from_str("150.00").map_err(|e| anyhow::anyhow!(e))?),
        volume: Decimal::from_str("1000000").map_err(|e| anyhow::anyhow!(e))?,
        is_final: true,
    };
    feed.push_candle(initial_candle.clone());

    let market_store = Arc::new(
        SqliteMarketStore::new_with_path(Some(root_path.clone()))
            .map_err(|e| anyhow::anyhow!("Failed to init SqliteMarketStore: {}", e))?,
    );
    let strategy_store = Arc::new(
        SqliteStrategyStore::new_with_path(Some(root_path.clone()))
            .map_err(|e| anyhow::anyhow!("Failed to init SqliteStrategyStore: {}", e))?,
    );

    // 注入历史行情，供回测使用
    let stock_identity = okane_core::common::Stock {
        symbol: "AAPL".to_string(),
        exchange: None,
    };
    market_store
        .save_candles(
            &stock_identity,
            okane_core::common::TimeFrame::Minute1,
            std::slice::from_ref(&initial_candle),
        )
        .await
        .map_err(|e| anyhow::anyhow!("Save 1m history: {}", e))?;
    market_store
        .save_candles(
            &stock_identity,
            okane_core::common::TimeFrame::Day1,
            std::slice::from_ref(&initial_candle),
        )
        .await
        .map_err(|e| anyhow::anyhow!("Save 1d history: {}", e))?;

    let market = MarketImpl::new(feed.clone(), market_store);

    // 触发 Stock 聚合根创建并订阅 feed
    let stock = market
        .get_stock("AAPL")
        .await
        .map_err(|e| anyhow::anyhow!("Failed to init stock AAPL: {}", e))?;
    // 推送行情到 feed，后台任务会自动更新缓存
    feed.push_candle(initial_candle);

    // 轮询等待价格就绪，防止 submit_order 报错 No latest price available
    let mut price_ready = false;
    for _ in 0..20 {
        if stock.current_price().map(|p| p.is_some()).unwrap_or(false) {
            price_ready = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    if !price_ready {
        return Err(anyhow::anyhow!(
            "Stock AAPL price failed to become ready in time"
        ));
    }

    let engine_builder = Arc::new(EngineFactory::new(market.clone()));
    let account_manager = Arc::new(AccountManager::default());
    account_manager.ensure_account_exists(
        okane_core::trade::entity::AccountId("SysAcct_1".to_string()),
        rust_decimal::Decimal::new(10_000_000, 2), // $100k test money
    );
    account_manager.ensure_account_exists(
        okane_core::trade::entity::AccountId("trader_01".to_string()),
        rust_decimal::Decimal::new(10_000_000, 2), // $100k test money
    );
    let pending_port = Arc::new(
        okane_store::pending_order_sqlx::SqlitePendingOrderStore::new_with_path(Some(root_path))
            .map_err(|e| anyhow::anyhow!("Failed to init SqlitePendingOrderStore: {}", e))?,
    );
    let matcher = std::sync::Arc::new(LocalMatchEngine::new(rust_decimal::Decimal::ZERO));
    let trade_service = Arc::new(TradeService::new(
        account_manager,
        matcher,
        market.clone(),
        pending_port,
        Arc::new(okane_core::common::time::RealTimeProvider),
    ));

    let engine_builder_factory = Arc::new(|m: Arc<dyn okane_core::market::port::Market>| {
        Arc::new(okane_engine::factory::EngineFactory::new(m))
            as Arc<dyn okane_core::engine::port::EngineBuilder>
    });

    let algo_service = Arc::new(okane_trade::algo::AlgoOrderService::new(
        trade_service.clone(),
        Arc::new(okane_core::common::time::RealTimeProvider),
    ));
    let indicator_service = Arc::new(okane_market::indicator::MarketIndicatorService::new(
        market.clone(),
    ));
    let app_config = Arc::new(okane_core::config::AppConfig::default());

    let backtest_runner = Arc::new(okane_manager::backtest::BacktestRunner::new(
        market.clone(),
        engine_builder_factory,
        Arc::new(TestBacktestEnvironmentFactory),
    ));

    let strategy_manager = StrategyManager::new(okane_manager::strategy::StrategyManagerParams {
        store: strategy_store.clone(),
        engine_builder: engine_builder as Arc<dyn okane_core::engine::port::EngineBuilder>,
        trade_port: trade_service.clone(),
        algo_port: algo_service.clone(),
        indicator_service: indicator_service.clone(),
        time_provider: Arc::new(okane_core::common::time::RealTimeProvider),
        notifier_factory: Arc::new(NoopNotifierFactory),
        log_port: strategy_store,
    });

    let state = AppState {
        strategy_manager: strategy_manager.clone(), // Fix potential missing clone if needed
        trade_port: trade_service,
        algo_port: algo_service,
        indicator_service,
        system_store: system_store.clone(),
        market_port: market,
        backtest_runner,
        app_config,
        session_cache: Arc::new(dashmap::DashMap::new()),
    };

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| anyhow::anyhow!("Failed to bind TcpListener: {}", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| anyhow::anyhow!("Failed to get local addr: {}", e))?
        .port();
    let addr = format!("http://127.0.0.1:{}", port);

    let router = okane_api::server::build_app(state);

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, router).await {
            tracing::error!("Server error: {}", e);
        }
    });

    let mut ready = false;
    for _ in 0..10 {
        if std::net::TcpStream::connect(format!("127.0.0.1:{}", port)).is_ok() {
            ready = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    if !ready {
        return Err(anyhow::anyhow!(
            "Test server failed to start on port {}",
            port
        ));
    }

    Ok((addr, system_store, tmp_dir, vec![stock], feed))
}

#[macro_export]
macro_rules! assert_post {
    ($client:expr, $url:expr, $token:expr, $body:expr, $status:expr) => {{
        let mut req = $client.post($url);
        if let Some(t) = $token {
            req = req.bearer_auth(t);
        }
        let res = req
            .json($body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send POST request: {}", e))?;
        let current_status = res.status();
        if current_status != $status {
            let text = res
                .text()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to get response text: {}", e))?;
            panic!(
                "post {} failed: expected {}, got {}. body: {}",
                $url, $status, current_status, text
            );
        }
        res
    }};
}

#[macro_export]
macro_rules! assert_get {
    ($client:expr, $url:expr, $token:expr, $status:expr) => {{
        let mut req = $client.get($url);
        if let Some(t) = $token {
            req = req.bearer_auth(t);
        }
        let res = req
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send GET request: {}", e))?;
        let current_status = res.status();
        if current_status != $status {
            let text = res
                .text()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to get response text: {}", e))?;
            panic!(
                "get {} failed: expected {}, got {}. body: {}",
                $url, $status, current_status, text
            );
        }
        res
    }};
}

#[macro_export]
macro_rules! assert_put {
    ($client:expr, $url:expr, $token:expr, $body:expr, $status:expr) => {{
        let mut req = $client.put($url);
        if let Some(t) = $token {
            req = req.bearer_auth(t);
        }
        let res = req
            .json($body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send PUT request: {}", e))?;
        let current_status = res.status();
        if current_status != $status {
            let text = res
                .text()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to get response text: {}", e))?;
            panic!(
                "put {} failed: expected {}, got {}. body: {}",
                $url, $status, current_status, text
            );
        }
        res
    }};
}

#[macro_export]
macro_rules! assert_delete {
    ($client:expr, $url:expr, $token:expr, $status:expr) => {{
        let mut req = $client.delete($url);
        if let Some(t) = $token {
            req = req.bearer_auth(t);
        }
        let res = req
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send DELETE request: {}", e))?;
        let current_status = res.status();
        if current_status != $status {
            let text = res
                .text()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to get response text: {}", e))?;
            panic!(
                "delete {} failed: expected {}, got {}. body: {}",
                $url, $status, current_status, text
            );
        }
        res
    }};
}
