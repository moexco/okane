use okane_core::trade::entity::{Order, OrderDirection, OrderStatus, Trade};
use okane_core::trade::port::MatcherPort;
use rust_decimal::Decimal;

/// # Summary
/// 针对测试和纸面模拟盘环境的内存级撮合引擎。
/// 接收逻辑委托单并执行基于当前（虚拟）价格的简化成交判定。
pub struct LocalMatchEngine {
    commission_rate: Decimal,
}

impl LocalMatchEngine {
    pub fn new(commission_rate: Decimal) -> Self {
        Self { commission_rate }
    }
}

impl MatcherPort for LocalMatchEngine {
    fn execute_order(
        &self,
        order: &mut Order,
        current_market_price: Decimal,
        now_ms: i64,
    ) -> Option<Trade> {
        if order.status != OrderStatus::Pending && order.status != OrderStatus::Submitted {
            return None;
        }

        if let Some(limit_price) = order.price {
            match order.direction {
                OrderDirection::Buy => {
                    if current_market_price > limit_price {
                        return None;
                    }
                }
                OrderDirection::Sell => {
                    if current_market_price < limit_price {
                        return None;
                    }
                }
            }
        }

        let execute_price = current_market_price;
        let executed_volume = order.volume - order.filled_volume;
        
        let transaction_val = execute_price * executed_volume;
        let commission = transaction_val * self.commission_rate;

        order.filled_volume += executed_volume;
        order.status = OrderStatus::Filled;

        let trade = Trade {
            order_id: order.id.clone(),
            account_id: order.account_id.clone(),
            symbol: order.symbol.clone(),
            direction: order.direction,
            price: execute_price,
            volume: executed_volume,
            commission,
            timestamp: now_ms,
        };

        Some(trade)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use okane_core::trade::entity::{AccountId, OrderDirection, OrderId};
    use rust_decimal_macros::dec;

    #[test]
    fn test_execute_order_market_price() -> Result<(), Box<dyn std::error::Error>> {
        let matcher = LocalMatchEngine::new(dec!(0.001)); // 0.1% commission
        let mut order = Order::new(
            OrderId("order1".to_string()),
            AccountId("acc1".to_string()),
            "AAPL".to_string(),
            OrderDirection::Buy,
            None,
            dec!(100),
            1000,
        );

        let trade = matcher.execute_order(&mut order, dec!(150.0), 1001)
            .ok_or("Failed to execute order")?;

        assert_eq!(trade.price, dec!(150.0));
        assert_eq!(trade.volume, dec!(100));
        assert_eq!(trade.commission, dec!(15.0)); // 150 * 100 * 0.001 = 15
        assert_eq!(order.status, OrderStatus::Filled);
        assert_eq!(order.filled_volume, dec!(100));
        Ok(())
    }

    #[test]
    fn test_execute_order_limit_price() -> Result<(), Box<dyn std::error::Error>> {
        let matcher = LocalMatchEngine::new(dec!(0.001));
        let mut order = Order::new(
            OrderId("order1".to_string()),
            AccountId("acc1".to_string()),
            "AAPL".to_string(),
            OrderDirection::Buy,
            Some(dec!(145.0)),
            dec!(100),
            1000,
        );

        // Since market price is lower (better) than limit price, it will execute at market price.
        let trade = matcher.execute_order(&mut order, dec!(140.0), 1001)
            .ok_or("Failed to execute order")?;

        assert_eq!(trade.price, dec!(140.0));
        assert_eq!(trade.volume, dec!(100));
        assert_eq!(order.status, OrderStatus::Filled);
        Ok(())
    }

    #[test]
    fn test_already_filled_order() {
        let matcher = LocalMatchEngine::new(dec!(0.001));
        let mut order = Order::new(
            OrderId("order1".to_string()),
            AccountId("acc1".to_string()),
            "AAPL".to_string(),
            OrderDirection::Buy,
            None,
            dec!(100),
            1000,
        );
        order.status = OrderStatus::Filled;

        let trade = matcher.execute_order(&mut order, dec!(150.0), 1001);
        assert!(trade.is_none());
    }
}
