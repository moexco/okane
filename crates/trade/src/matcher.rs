use okane_core::trade::entity::{Order, OrderStatus, Trade};
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

        let execute_price = order.price.unwrap_or(current_market_price);
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
