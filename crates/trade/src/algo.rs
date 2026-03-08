use async_trait::async_trait;
use dashmap::DashMap;
use okane_core::trade::entity::{AccountId, AlgoOrder, OrderId, AlgoOrderStatus, Order, OrderDirection, AlgoType};
use okane_core::trade::port::{AlgoOrderPort, TradeError, TradePort};
use okane_core::market::entity::Candle;
use std::sync::Arc;
use okane_core::common::time::TimeProvider;

/// # Summary
/// 算法单管理与执行服务。
pub struct AlgoOrderService {
    /// 存放所有活跃算法单
    algo_orders: DashMap<OrderId, AlgoOrder>,
    /// 基础交易服务，用于下发子单
    trade_port: Arc<dyn TradePort>,
    /// 时间提供者
    time_provider: Arc<dyn TimeProvider>,
}

impl AlgoOrderService {
    pub fn new(trade_port: Arc<dyn TradePort>, time_provider: Arc<dyn TimeProvider>) -> Self {
        Self {
            algo_orders: DashMap::new(),
            trade_port,
            time_provider,
        }
    }

    /// 驱动算法单运行的方法。每当新 K 线到达时调用。
    pub async fn tick(&self, symbol: &str, candle: &Candle) -> Result<(), TradeError> {
        for mut entry in self.algo_orders.iter_mut() {
            let order = entry.value_mut();
            if order.symbol != symbol || order.status != AlgoOrderStatus::Running {
                continue;
            }

            // 根据算法类型执行逻辑
            if let AlgoType::Snipe { target_price, max_slippage: _ } = &order.algo {
                // 狙击单逻辑：如果当前价格达到或优于目标价，立即触发市价单
                if candle.close <= *target_price {
                    let sub_order = Order::new(
                        OrderId(format!("{}-child", order.id.0)),
                        order.account_id.clone(),
                        order.symbol.clone(),
                        OrderDirection::Buy,
                        None,
                        // 这里简单处理为一笔单子买完，实际上应该根据 params 决定
                        order.filled_volume, // 示例中暂未完善 volume 定义，先用占位
                        self.time_provider.now()
                            .map_err(|e| TradeError::InternalError(e.to_string()))?
                            .timestamp_millis()
                    );
                    self.trade_port.submit_order(sub_order).await?;
                    order.status = AlgoOrderStatus::Completed;
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
impl AlgoOrderPort for AlgoOrderService {
    async fn submit_algo_order(&self, order: AlgoOrder) -> Result<OrderId, TradeError> {
        let id = order.id.clone();
        self.algo_orders.insert(id.clone(), order);
        Ok(id)
    }

    async fn cancel_algo_order(&self, order_id: &OrderId) -> Result<(), TradeError> {
        if let Some(mut order) = self.algo_orders.get_mut(order_id) {
            order.status = AlgoOrderStatus::Canceled;
            Ok(())
        } else {
            Err(TradeError::AlgoOrderNotFound(order_id.0.clone()))
        }
    }

    async fn get_algo_order(&self, order_id: &OrderId) -> Result<Option<AlgoOrder>, TradeError> {
        Ok(self.algo_orders.get(order_id).map(|o| o.value().clone()))
    }

    async fn get_algo_orders(&self, account_id: &AccountId) -> Result<Vec<AlgoOrder>, TradeError> {
        Ok(self.algo_orders
            .iter()
            .filter(|o| o.value().account_id == *account_id)
            .map(|o| o.value().clone())
            .collect())
    }

    async fn update_algo_status(&self, order_id: &OrderId, status: AlgoOrderStatus) -> Result<(), TradeError> {
        if let Some(mut order) = self.algo_orders.get_mut(order_id) {
            order.status = status;
            Ok(())
        } else {
            Err(TradeError::AlgoOrderNotFound(order_id.0.clone()))
        }
    }
}
