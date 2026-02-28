use okane_core::trade::entity::{Order, OrderStatus, Trade};
use rust_decimal::Decimal;

/// # Summary
/// 针对测试和纸面模拟盘环境的内存级撮合引擎。
/// 接收逻辑委托单并执行基于当前（虚拟）价格的简化成交判定。
pub struct LocalMatchEngine;

impl Default for LocalMatchEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalMatchEngine {
    pub fn new() -> Self {
        Self
    }

    /// # Logic
    /// 执行一张独立的逻辑委托单的撮合计价计算。
    /// 鉴于我们系统后续会有 FakeClock 和 DataReplayer 驱动行情，目前这里的撮合逻辑采取最弱假设：
    /// **所有当前通过市价发来的单子，均以传入的 `current_market_price` 直接全量成交。**
    /// 包含简单的滑点/单边手续费测算（预留）。
    pub fn execute_order(
        &self,
        order: &mut Order,
        current_market_price: Decimal,
        now_ms: i64,
    ) -> Option<Trade> {
        if order.status != OrderStatus::Pending && order.status != OrderStatus::Submitted {
            return None;
        }

        // 以传进来的最新市价/收盘价为执行价格
        // todo: 如果之后传进来的是 Candle 高低点，也可以做限价单的拦截和滑点穿越检查。
        let execute_price = order.price.unwrap_or(current_market_price);

        // 我们简单假设无论多少量，在这里都是一笔直接完全吃下
        let executed_volume = order.volume - order.filled_volume;
        
        // 极简的手续费测算 (默认假设万一的佣金比例，双边收取)
        let commission_rate = Decimal::from_str_exact("0.0001").unwrap();
        let transaction_val = execute_price * executed_volume;
        let commission = transaction_val * commission_rate;

        // 更新此张订单为主控单里的状态
        order.filled_volume += executed_volume;
        order.status = OrderStatus::Filled;

        // 生成正式成交流水
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
