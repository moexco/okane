use async_trait::async_trait;
use dashmap::DashMap;
use okane_core::common::time::TimeProvider;
use okane_core::market::entity::Candle;
use okane_core::trade::entity::{
    AccountId, AlgoOrder, AlgoOrderStatus, AlgoType, Order, OrderDirection, OrderId,
};
use okane_core::trade::port::{AlgoOrderPort, TradeError, TradePort};
use std::sync::Arc;

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
    /// # Logic
    /// Create an algo order service backed by the provided trade port and time provider.
    ///
    /// # Arguments
    /// * `trade_port` - Trade submission port for spawned child orders.
    /// * `time_provider` - Clock source used for child order timestamps.
    ///
    /// # Returns
    /// * `Self` - A new in-memory algo order service instance.
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
            if let AlgoType::Snipe {
                target_price,
                max_slippage: _,
            } = &order.algo
            {
                // 狙击单逻辑：如果当前价格达到或优于目标价，立即触发市价单
                if candle.close <= *target_price {
                    let remaining_volume = order.requested_volume - order.filled_volume;
                    if remaining_volume <= rust_decimal::Decimal::ZERO {
                        return Err(TradeError::AlgoOrderError(format!(
                            "algo order {} has no remaining volume",
                            order.id.0
                        )));
                    }

                    let sub_order = Order::new(
                        OrderId(format!("{}-child", order.id.0)),
                        order.account_id.clone(),
                        order.symbol.clone(),
                        OrderDirection::Buy,
                        None,
                        remaining_volume,
                        self.time_provider
                            .now()
                            .map_err(|e| TradeError::InternalError(e.to_string()))?
                            .timestamp_millis(),
                    );
                    self.trade_port.submit_order(sub_order).await?;
                    order.filled_volume = order.requested_volume;
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
        Ok(self
            .algo_orders
            .iter()
            .filter(|o| o.value().account_id == *account_id)
            .map(|o| o.value().clone())
            .collect())
    }

    async fn update_algo_status(
        &self,
        order_id: &OrderId,
        status: AlgoOrderStatus,
    ) -> Result<(), TradeError> {
        if let Some(mut order) = self.algo_orders.get_mut(order_id) {
            order.status = status;
            Ok(())
        } else {
            Err(TradeError::AlgoOrderNotFound(order_id.0.clone()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use okane_core::common::time::FakeClockProvider;
    use okane_core::test_utils::SpyTradePort;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    #[tokio::test]
    async fn test_submit_and_get_algo_order() -> anyhow::Result<()> {
        let trade_port = Arc::new(SpyTradePort::new());
        let time_provider = Arc::new(FakeClockProvider::new(chrono::Utc::now()));
        let service = AlgoOrderService::new(trade_port, time_provider);

        let account_id = AccountId("test_acct".into());
        let order_id = OrderId("algo_01".into());
        let order = AlgoOrder::new(
            order_id.clone(),
            account_id.clone(),
            "AAPL".into(),
            AlgoType::Snipe {
                target_price: dec!(150.0),
                max_slippage: dec!(0.1),
            },
            dec!(10),
            1000,
        );

        service.submit_algo_order(order).await?;

        let retrieved = service
            .get_algo_order(&order_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("AlgoOrder not found"))?;
        assert_eq!(retrieved.symbol, "AAPL");

        let all = service.get_algo_orders(&account_id).await?;
        assert_eq!(all.len(), 1);

        Ok(())
    }

    #[tokio::test]
    async fn test_cancel_algo_order() -> anyhow::Result<()> {
        let trade_port = Arc::new(SpyTradePort::new());
        let time_provider = Arc::new(FakeClockProvider::new(chrono::Utc::now()));
        let service = AlgoOrderService::new(trade_port, time_provider);

        let order_id = OrderId("algo_01".into());
        let order = AlgoOrder::new(
            order_id.clone(),
            AccountId("test".into()),
            "AAPL".into(),
            AlgoType::Snipe {
                target_price: dec!(150.0),
                max_slippage: dec!(0.1),
            },
            dec!(10),
            1000,
        );

        service.submit_algo_order(order).await?;
        service.cancel_algo_order(&order_id).await?;

        let retrieved = service
            .get_algo_order(&order_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("AlgoOrder not found"))?;
        assert_eq!(retrieved.status, AlgoOrderStatus::Canceled);

        Ok(())
    }

    #[tokio::test]
    async fn test_snipe_algo_trigger() -> anyhow::Result<()> {
        let spy_trade = Arc::new(SpyTradePort::new());
        let time_provider = Arc::new(FakeClockProvider::new(chrono::Utc::now()));

        let service = AlgoOrderService::new(spy_trade.clone(), time_provider);

        let order_id = OrderId("snipe_01".into());
        let order = AlgoOrder {
            id: order_id.clone(),
            account_id: AccountId("test".into()),
            symbol: "AAPL".into(),
            algo: AlgoType::Snipe {
                target_price: dec!(100.0),
                max_slippage: dec!(0.1),
            },
            status: AlgoOrderStatus::Running,
            requested_volume: dec!(10),
            filled_volume: Decimal::ZERO,
            created_at: 1000,
        };

        service.submit_algo_order(order).await?;

        // 1. Price above target - no trigger
        let candle_high = Candle {
            time: chrono::Utc::now(),
            open: dec!(105),
            high: dec!(106),
            low: dec!(104),
            close: dec!(105),
            adj_close: None,
            volume: dec!(100),
            is_final: true,
        };
        service.tick("AAPL", &candle_high).await?;
        assert_eq!(spy_trade.get_submitted_orders()?.len(), 0);
        let retrieved = service
            .get_algo_order(&order_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("AlgoOrder not found"))?;
        assert_eq!(retrieved.status, AlgoOrderStatus::Running);

        // 2. Price hits target - trigger!
        let candle_hit = Candle {
            time: chrono::Utc::now(),
            open: dec!(101),
            high: dec!(102),
            low: dec!(99),
            close: dec!(100),
            adj_close: None,
            volume: dec!(100),
            is_final: true,
        };
        service.tick("AAPL", &candle_hit).await?;

        let submitted = spy_trade.get_submitted_orders()?;
        assert_eq!(submitted.len(), 1);
        assert_eq!(submitted[0].symbol, "AAPL");
        assert_eq!(submitted[0].direction, OrderDirection::Buy);
        assert_eq!(submitted[0].volume, dec!(10));

        let retrieved = service
            .get_algo_order(&order_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("AlgoOrder not found"))?;
        assert_eq!(retrieved.status, AlgoOrderStatus::Completed);

        Ok(())
    }
}
