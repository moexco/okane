use async_trait::async_trait;
use okane_core::common::time::FakeClockProvider;
use okane_core::market::port::{Market, Stock};
use okane_core::trade::entity::{AccountId, AccountSnapshot, Trade};
use okane_core::trade::port::TradePort;
use okane_manager::backtest::{
    BacktestEnvironment, BacktestEnvironmentFactory, BacktestRequest, BacktestResultCollector,
};
use okane_manager::strategy::ManagerError;
use okane_market::history::BacktestMarket;
use okane_market::indicator::MarketIndicatorService;
use okane_trade::algo::AlgoOrderService;
use okane_trade::service::TradeService;
use okane_trade::trade_log::TradeLog;
use rust_decimal::Decimal;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};

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
        let mut guard = self
            .inner
            .lock()
            .map_err(|e| okane_core::market::error::MarketError::Unknown(e.to_string()))?;
        if guard.is_some() {
            return Err(okane_core::market::error::MarketError::Unknown(
                "lazy market already initialized".to_string(),
            ));
        }
        *guard = Some(market);
        Ok(())
    }
}

#[async_trait]
impl Market for LazyMarket {
    async fn get_stock(
        &self,
        symbol: &str,
    ) -> Result<Arc<dyn Stock>, okane_core::market::error::MarketError> {
        let market = {
            let guard = self
                .inner
                .lock()
                .map_err(|e| okane_core::market::error::MarketError::Unknown(e.to_string()))?;
            guard.as_ref().cloned().ok_or_else(|| {
                okane_core::market::error::MarketError::Unknown(
                    "lazy market not initialized".to_string(),
                )
            })?
        };
        market.get_stock(symbol).await
    }

    async fn search_symbols(
        &self,
        query: &str,
    ) -> Result<Vec<okane_core::store::port::StockMetadata>, okane_core::market::error::MarketError>
    {
        let market = {
            let guard = self
                .inner
                .lock()
                .map_err(|e| okane_core::market::error::MarketError::Unknown(e.to_string()))?;
            guard.as_ref().cloned().ok_or_else(|| {
                okane_core::market::error::MarketError::Unknown(
                    "lazy market not initialized".to_string(),
                )
            })?
        };
        market.search_symbols(query).await
    }
}

struct DefaultBacktestResultCollector {
    trade_port: Arc<dyn TradePort>,
    trade_log: Arc<TradeLog>,
}

#[async_trait]
impl BacktestResultCollector for DefaultBacktestResultCollector {
    async fn final_snapshot(
        &self,
        account_id: &AccountId,
    ) -> Result<AccountSnapshot, ManagerError> {
        Ok(self.trade_port.get_account(account_id.clone()).await?)
    }

    async fn drain_trades(&self) -> Result<Vec<Trade>, ManagerError> {
        Ok(self.trade_log.drain()?)
    }
}

pub struct DefaultBacktestEnvironmentFactory;

#[async_trait]
impl BacktestEnvironmentFactory for DefaultBacktestEnvironmentFactory {
    async fn create(
        &self,
        req: &BacktestRequest,
        source_stock: Arc<dyn Stock>,
    ) -> Result<BacktestEnvironment, ManagerError> {
        let fake_clock = Arc::new(FakeClockProvider::new(req.start));
        let account_store = Arc::new(
            okane_store::account::SqliteAccountStore::new().map_err(|e| {
                ManagerError::Trade(okane_core::trade::port::TradeError::InternalError(
                    e.to_string(),
                ))
            })?,
        );
        let backtest_account_id = AccountId(format!("backtest_{}", uuid::Uuid::new_v4()));
        let pending_port = Arc::new(
            okane_store::pending_order_sqlx::SqlitePendingOrderStore::new().map_err(|e| {
                ManagerError::Trade(okane_core::trade::port::TradeError::InternalError(
                    e.to_string(),
                ))
            })?,
        );
        let matcher = Arc::new(okane_trade::matcher::LocalMatchEngine::new(Decimal::ZERO));
        let trade_log = Arc::new(TradeLog::new());
        let lazy_market = Arc::new(LazyMarket::new());
        let candle_counter = Arc::new(AtomicUsize::new(0));

        let trade_service = Arc::new(
            TradeService::new(
                account_store,
                matcher,
                lazy_market.clone(),
                pending_port,
                fake_clock.clone(),
            )
            .with_trade_log(trade_log.clone()),
        );
        trade_service
            .ensure_account(backtest_account_id.clone(), req.initial_balance)
            .await?;

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
            ManagerError::Engine(okane_core::engine::error::EngineError::Plugin(format!(
                "failed to initialize lazy market: {}",
                e
            )))
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
            result_collector: Arc::new(DefaultBacktestResultCollector {
                trade_port: trade_service.clone(),
                trade_log,
            }),
            candle_counter,
        })
    }
}
