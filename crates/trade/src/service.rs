use crate::account::AccountManager;
use crate::matcher::LocalMatchEngine;
use async_trait::async_trait;
use okane_core::market::port::Market;
use okane_core::trade::entity::{AccountId, AccountSnapshot, Order, OrderDirection, OrderId, OrderStatus};
use okane_core::trade::port::{TradeError, TradePort};
use rust_decimal::Decimal;
use std::sync::Arc;

/// # Summary
/// `TradeService` 是纸面交易环境和内盘 OMS 的入口调度者，
/// 实现了 `TradePort`，对接着最原始的逻辑 AccountManager 和 LocalMatchEngine。
pub struct TradeService {
    account_manager: Arc<AccountManager>,
    matcher: LocalMatchEngine,
    /// 用于获取当前标的市场行情的抽象指针 (用于模拟成交)
    market: Arc<dyn Market>,
}

impl TradeService {
    pub fn new(account_manager: Arc<AccountManager>, market: Arc<dyn Market>) -> Self {
        Self {
            account_manager,
            matcher: LocalMatchEngine::new(),
            market,
        }
    }
}

#[async_trait]
impl TradePort for TradeService {
    /// # Logic
    /// 1. 获取目标账户锁。
    /// 2. 如果是买单，计算所需的预估冻结金额 (如果市价单且没有预估金额，则按最新价 * 倍数 兜底)。
    /// 3. 从账户扣划并冻结。如果可用金额不足抛错。
    /// 4. 提交订单到本地撮合器（由于是模拟回测环境，直接触发立即执行）。
    /// 5. 撮合器吐出 Trade，账户按 Trade 真实价格和数量扣减冻结资金及更新持仓。
    async fn submit_order(&self, mut order: Order) -> Result<OrderId, TradeError> {
        let order_id = order.id.clone();
        
        let account_lock = self.account_manager.get_account(&order.account_id)?;
        let mut acct = account_lock.write().await;

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
            acct.freeze_funds(est_req_funds)?;
        }

        // 发向撮合引擎模拟立刻触发
        // (在真实的系统中，这一步是将订单投递到异步 Channel，但回测沙盒为了防止乱序支持同步吃单)
        let now_ms = chrono::Utc::now().timestamp_millis();
        order.status = OrderStatus::Submitted;
        
        if let Some(trade) = self.matcher.execute_order(&mut order, latest_price, now_ms) {
            // 收到正式的成交流水，释放多余的冻结款并做真正的扣减
            
            // 粗略模拟：
            // 如果是买单，扣除对应成交额与手续费；并由于成交了释放占用的冻结锁定
            if trade.direction == OrderDirection::Buy {
                let actual_cost = trade.price * trade.volume + trade.commission;
                // 注意这里只扣除了这笔成交占用的逻辑金额，其余解冻
                acct.deduct_funds(actual_cost);
                let over_frozen = est_req_funds - actual_cost;
                if over_frozen > Decimal::ZERO {
                    acct.unfreeze_funds(over_frozen);
                }
            } else {
                // 如果是卖空，得到现金，并扣除手续费
                let actual_gain = trade.price * trade.volume - trade.commission;
                acct.add_funds(actual_gain);
            }

            // 更新账户的对应持仓数量及均价（买入+，卖出-）
            let position_delta = if trade.direction == OrderDirection::Buy {
                trade.volume
            } else {
                -trade.volume
            };
            acct.update_position(&trade.symbol, position_delta, trade.price);
        }

        Ok(order_id)
    }

    async fn cancel_order(&self, _order_id: OrderId) -> Result<(), TradeError> {
        // 本地环境由于所有单子目前都是同步立即戳合的，所以撤单在此简易版中通常为已完成抛错
        Err(TradeError::InvalidOrderStatus)
    }

    async fn get_account(&self, account_id: AccountId) -> Result<AccountSnapshot, TradeError> {
        self.account_manager.snapshot(&account_id).await
    }
}
