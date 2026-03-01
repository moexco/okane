use async_trait::async_trait;
use okane_core::market::entity::Candle;
use okane_core::market::port::Market;
use okane_core::trade::entity::{AccountId, AccountSnapshot, Order, OrderDirection, OrderId, OrderStatus};
use okane_core::trade::port::{AccountPort, BacktestTradePort, MatcherPort, TradeError, TradePort, PendingOrderPort};
use rust_decimal::Decimal;
use std::sync::Arc;

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
}

impl TradeService {
    pub fn new(account_port: Arc<dyn AccountPort>, matcher: Arc<dyn MatcherPort>, market: Arc<dyn Market>, pending_port: Arc<dyn PendingOrderPort>) -> Self {
        Self {
            account_port,
            matcher,
            market,
            pending_port,
        }
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
            TradeError::BrokerIntegrationError(format!("无法获取行情: {}", e))
        })?;
        
        // FIXME: 如果是停牌、退市或者根本没有最新价，直接拒绝
        let current_p = stock.current_price().ok_or_else(|| TradeError::InternalError("股票暂无最新报价".into()))?;
        let latest_price = Decimal::from_f64_retain(current_p)
            .ok_or_else(|| TradeError::InternalError("市价非有效精度数值".into()))?;

        // 预估单价 (限价取限价，市价取最新价)
        let est_price = order.price.unwrap_or(latest_price);
        let est_req_funds = est_price * order.volume;

        // 如果是多头买单，先冻结需要的总现金款
        if order.direction == OrderDirection::Buy {
            self.account_port.freeze_funds(&order.account_id, est_req_funds).await?;
        }

        // 如果是市价单 (price == None)，立刻尝试撮合。
        // 如果是限价单，先放入 Pending 队列等待下一个 Tick。
        if order.price.is_none() {
            let now_ms = chrono::Utc::now().timestamp_millis();
            order.status = OrderStatus::Submitted;
            
            if let Some(trade) = self.matcher.execute_order(&mut order, latest_price, now_ms) {
                self.account_port.process_trade(&order.account_id, &trade, est_req_funds).await?;
            }
        } else {
            // 限价单，等待未来穿越
            order.status = OrderStatus::Pending;
            self.pending_port.save(order).await?;
        }

        Ok(order_id)
    }

    async fn cancel_order(&self, order_id: OrderId) -> Result<(), TradeError> {
        if let Some(mut order) = self.pending_port.remove(&order_id).await? {
            order.status = OrderStatus::Canceled;
            // 还需要退回冻结资金
            if order.direction == OrderDirection::Buy && let Some(price) = order.price {
                let amount = price * (order.volume - order.filled_volume);
                self.account_port.unfreeze_funds(&order.account_id, amount).await?;
            }
            Ok(())
        } else {
            Err(TradeError::OrderNotFound("Order not found or already filled".into()))
        }
    }

    async fn get_account(&self, account_id: AccountId) -> Result<AccountSnapshot, TradeError> {
        self.account_port.snapshot(&account_id).await
    }

    async fn get_orders(&self, account_id: &AccountId) -> Result<Vec<Order>, TradeError> {
        self.pending_port.get_by_account(account_id).await
    }

    async fn get_order(&self, order_id: &OrderId) -> Result<Option<Order>, TradeError> {
        self.pending_port.get(order_id).await
    }
}

#[async_trait]
impl BacktestTradePort for TradeService {
    async fn tick(&self, symbol: &str, candle: &Candle) -> Result<(), TradeError> {
        let mut filled_orders = Vec::new();

        let high = rust_decimal::Decimal::from_f64_retain(candle.high).unwrap_or(rust_decimal::Decimal::ZERO);
        let low = rust_decimal::Decimal::from_f64_retain(candle.low).unwrap_or(rust_decimal::Decimal::ZERO);

        let pending = self.pending_port.get_by_symbol(symbol).await?;

        for mut order in pending {
            if let Some(limit_price) = order.price {
                // 击穿判定
                let is_hit = (order.direction == OrderDirection::Buy && low <= limit_price) || 
                             (order.direction == OrderDirection::Sell && high >= limit_price);
                
                if is_hit {
                    // 成交取限价或者由于跳空引起的开盘价劣势
                    let exec_price = limit_price;
                    let now_ms = candle.time.timestamp_millis();
                    
                    if let Some(trade) = self.matcher.execute_order(&mut order, exec_price, now_ms) {
                        let est_req_funds = limit_price * trade.volume; // 原本冻结的总数
                        if let Err(e) = self.account_port.process_trade(&order.account_id, &trade, est_req_funds).await {
                            tracing::warn!("Backtest tick: Failed process trade on account {}: {}", order.id.0, e);
                        }
                    }
                    filled_orders.push(order.id.clone());
                }
            }
        }

        for id in filled_orders {
            self.pending_port.remove(&id).await?;
        }

        Ok(())
    }
}
