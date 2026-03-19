use async_trait::async_trait;
use okane_core::common::time::TimeProvider;
use okane_core::market::entity::Candle;
use okane_core::market::port::Market;
use okane_core::trade::entity::{
    AccountId, AccountSnapshot, Order, OrderDirection, OrderId, OrderStatus,
};
use okane_core::trade::port::{
    AccountPort, BacktestTradePort, MatcherPort, PendingOrderPort, TradeError, TradePort,
};
use std::sync::Arc;
use std::sync::RwLock;

use crate::trade_log::TradeLog;

/// # Summary
/// `TradeService` 是纸面交易环境和内盘 OMS 的入口调度者，
/// 实现了 `TradePort`，对接着最原始的逻辑 AccountManager 和 LocalMatchEngine。
pub struct TradeService {
    account_port: Arc<dyn AccountPort>,
    matcher: Arc<dyn MatcherPort>,
    /// 用于获取当前标的市场行情的抽象指针 (用于模拟成交)
    market: Arc<dyn Market>,
    /// 活动订单持久化端口
    pending_port: Arc<dyn PendingOrderPort>,
    /// 算法单服务
    algo_service: RwLock<Option<Arc<crate::algo::AlgoOrderService>>>,
    /// 逻辑时钟源，回测时由 FakeClockProvider 提供，实盘为 RealTimeProvider
    time_provider: Arc<dyn TimeProvider>,
    /// 可选的交易事件收集器 — 记录所有成交，用于回测结果提取
    trade_log: Option<Arc<TradeLog>>,
}

impl TradeService {
    fn is_active_order_status(status: OrderStatus) -> bool {
        matches!(
            status,
            OrderStatus::Pending | OrderStatus::Submitted | OrderStatus::PartialFilled
        )
    }

    fn estimate_buy_funds(
        &self,
        price: rust_decimal::Decimal,
        volume: rust_decimal::Decimal,
    ) -> rust_decimal::Decimal {
        let commission = self.matcher.estimate_commission(price, volume);
        price * volume + commission
    }

    async fn mark_to_market_snapshot(
        &self,
        mut snapshot: AccountSnapshot,
    ) -> Result<AccountSnapshot, TradeError> {
        let mut positions_market_value = rust_decimal::Decimal::ZERO;
        for position in &snapshot.positions {
            let stock = self.market.get_stock(&position.symbol).await.map_err(|e| {
                TradeError::BrokerIntegrationError(format!("Failed to get market data: {}", e))
            })?;
            let latest_price = stock
                .current_price()
                .map_err(|e| TradeError::InternalError(e.to_string()))?
                .ok_or_else(|| {
                    TradeError::InternalError(format!(
                        "No latest price available for stock {}",
                        position.symbol
                    ))
                })?;
            positions_market_value += position.volume * latest_price;
        }
        snapshot.total_equity =
            snapshot.available_balance + snapshot.frozen_balance + positions_market_value;
        Ok(snapshot)
    }

    pub fn new(
        account_port: Arc<dyn AccountPort>,
        matcher: Arc<dyn MatcherPort>,
        market: Arc<dyn Market>,
        pending_port: Arc<dyn PendingOrderPort>,
        time_provider: Arc<dyn TimeProvider>,
    ) -> Self {
        Self {
            account_port,
            matcher,
            market,
            pending_port,
            algo_service: RwLock::new(None),
            time_provider,
            trade_log: None,
        }
    }

    pub fn with_algo_service(
        self,
        algo_service: Arc<crate::algo::AlgoOrderService>,
    ) -> Result<Self, TradeError> {
        self.set_algo_service(algo_service)?;
        Ok(self)
    }

    /// # Logic
    /// Replace the currently configured algo order service.
    ///
    /// # Arguments
    /// * `algo_service` - Algo order service used during backtest ticks.
    ///
    /// # Returns
    /// None.
    pub fn set_algo_service(
        &self,
        algo_service: Arc<crate::algo::AlgoOrderService>,
    ) -> Result<(), TradeError> {
        let mut guard = self
            .algo_service
            .write()
            .map_err(|e| TradeError::InternalError(format!("algo service lock poisoned: {}", e)))?;
        *guard = Some(algo_service);
        Ok(())
    }

    /// 设置交易事件收集器。回测场景下使用。
    pub fn with_trade_log(mut self, trade_log: Arc<TradeLog>) -> Self {
        self.trade_log = Some(trade_log);
        self
    }
}

#[async_trait]
impl TradePort for TradeService {
    /// # Logic
    /// 1. 如果是买单，计算所需的预估冻结金额 (如果市价单且没有预估金额，则按最新价 * 倍数 兜底)。
    /// 2. 从账户端口请求冻结。如果可用金额不足抛错。
    /// 3. 提交订单到本地撮合端口（由于是模拟回测环境，直接触发立即执行）。
    /// 4. 撮合器吐出 Trade，账户端口按 Trade 真实价格和数量扣减冻结资金及更新持仓。
    async fn submit_order(&self, mut order: Order) -> Result<OrderId, TradeError> {
        let order_id = order.id.clone();

        let stock = self.market.get_stock(&order.symbol).await.map_err(|e| {
            TradeError::BrokerIntegrationError(format!("Failed to get market data: {}", e))
        })?;

        // 检查标的状态：停牌、退市等情况直接拒绝
        let status = stock.status();
        if status != okane_core::market::port::StockStatus::Online {
            return Err(TradeError::InternalError(format!(
                "Stock status is {:?}, order rejected",
                status
            )));
        }

        let latest_price = stock
            .current_price()
            .map_err(|e| TradeError::InternalError(e.to_string()))?
            .ok_or_else(|| {
                TradeError::InternalError("No latest price available for stock".into())
            })?;

        // 预估单价 (限价单取限价，市价单取市场最新的成交价进行预估撮合)。
        // OK: Intentional business fallback for Market Orders
        let est_price = order.price.unwrap_or(latest_price);
        let est_req_funds = self.estimate_buy_funds(est_price, order.volume);

        // 如果是多头买单，先冻结需要的总现金款
        if order.direction == OrderDirection::Buy {
            self.account_port
                .freeze_funds(&order.account_id, est_req_funds)
                .await?;
        }

        // 如果是市价单 (price == None)，立刻尝试撮合。
        // 如果是限价单，先放入 Pending 队列等待下一个 Tick。
        if order.price.is_none() {
            let now_ms = self
                .time_provider
                .now()
                .map_err(|e| TradeError::InternalError(e.to_string()))?
                .timestamp_millis();
            order.status = OrderStatus::Submitted;

            if let Some(trade) = self.matcher.execute_order(&mut order, latest_price, now_ms) {
                if let Some(log) = &self.trade_log {
                    log.record(&trade)
                        .map_err(|e| TradeError::InternalError(e.to_string()))?;
                }
                self.account_port
                    .process_trade(&order.account_id, &trade, est_req_funds)
                    .await?;
            }

            if Self::is_active_order_status(order.status) {
                self.pending_port.save(order).await?;
            }
        } else {
            // 限价单，等待未来穿越
            order.status = OrderStatus::Pending;
            self.pending_port.save(order).await?;
        }

        Ok(order_id)
    }

    async fn cancel_order(&self, order_id: OrderId) -> Result<(), TradeError> {
        let order =
            self.pending_port.get(&order_id).await?.ok_or_else(|| {
                TradeError::OrderNotFound("order not found or already filled".into())
            })?;

        if !Self::is_active_order_status(order.status) {
            return Err(TradeError::InvalidOrderStatus);
        }

        let mut order =
            self.pending_port.remove(&order_id).await?.ok_or_else(|| {
                TradeError::OrderNotFound("order not found or already filled".into())
            })?;
        order.status = OrderStatus::Canceled;
        // 还需要退回冻结资金
        if order.direction == OrderDirection::Buy
            && let Some(price) = order.price
        {
            let remaining_volume = order.volume - order.filled_volume;
            let amount = self.estimate_buy_funds(price, remaining_volume);
            self.account_port
                .unfreeze_funds(&order.account_id, amount)
                .await?;
        }
        Ok(())
    }

    async fn get_account(&self, account_id: AccountId) -> Result<AccountSnapshot, TradeError> {
        let snapshot = self.account_port.snapshot(&account_id).await?;
        self.mark_to_market_snapshot(snapshot).await
    }

    async fn get_orders(&self, account_id: &AccountId) -> Result<Vec<Order>, TradeError> {
        self.pending_port.get_by_account(account_id).await
    }

    async fn get_order(&self, order_id: &OrderId) -> Result<Option<Order>, TradeError> {
        self.pending_port.get(order_id).await
    }

    async fn ensure_account(
        &self,
        account_id: AccountId,
        initial_balance: rust_decimal::Decimal,
    ) -> Result<(), TradeError> {
        self.account_port
            .ensure_account(&account_id, initial_balance)
            .await
    }
}

#[async_trait]
impl BacktestTradePort for TradeService {
    async fn tick(&self, symbol: &str, candle: &Candle) -> Result<(), TradeError> {
        // 首先驱动算法单
        let algo_service = self
            .algo_service
            .read()
            .map_err(|e| TradeError::InternalError(format!("algo service lock poisoned: {}", e)))?
            .clone();
        if let Some(algo) = algo_service {
            algo.tick(symbol, candle).await?;
        }

        let high = candle.high;
        let low = candle.low;

        let pending = self.pending_port.get_by_symbol(symbol).await?;

        for mut order in pending {
            if let Some(limit_price) = order.price {
                // 击穿判定
                let is_hit = (order.direction == OrderDirection::Buy && low <= limit_price)
                    || (order.direction == OrderDirection::Sell && high >= limit_price);

                if is_hit {
                    // 成交取限价或者由于跳空引起的开盘价劣势
                    let exec_price = limit_price;
                    let now_ms = candle.time.timestamp_millis();

                    if let Some(trade) = self.matcher.execute_order(&mut order, exec_price, now_ms)
                    {
                        if let Some(log) = &self.trade_log {
                            log.record(&trade)
                                .map_err(|e| TradeError::InternalError(e.to_string()))?;
                        }
                        let est_req_funds = self.estimate_buy_funds(limit_price, trade.volume);
                        self.account_port
                            .process_trade(&order.account_id, &trade, est_req_funds)
                            .await?;
                    }

                    // 核心修复：只有达到终态才移除，否则更新（部分成交）
                    if order.status == OrderStatus::Filled || order.status == OrderStatus::Canceled
                    {
                        self.pending_port.remove(&order.id).await?;
                    } else {
                        // 如果有部分成交或状态变更，更新持久化存储
                        self.pending_port.save(order).await?;
                    }
                }
            }
        }

        Ok(())
    }
}
