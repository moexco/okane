use okane_core::trade::entity::Trade;
use std::sync::Mutex;

/// # Summary
/// 交易事件收集器，记录所有成交。
///
/// 用于回测结束后提取完整的交易历史。生产环境也可用于审计日志。
///
/// # Invariants
/// - 线程安全: 内部使用 `Mutex` 保护写入。
/// - 仅追加、不可变: 收集器仅支持 `record` 和 `drain`，不支持单条删除。
pub struct TradeLog {
    trades: Mutex<Vec<Trade>>,
}

impl TradeLog {
    /// 创建一个空的交易收集器
    pub fn new() -> Self {
        Self {
            trades: Mutex::new(Vec::new()),
        }
    }

    /// 记录一笔成交
    pub fn record(&self, trade: &Trade) {
        let mut trades = self.trades.lock().unwrap_or_else(|e| e.into_inner());
        trades.push(trade.clone());
    }

    /// 取出所有已记录的成交并清空收集器
    pub fn drain(&self) -> Vec<Trade> {
        let mut trades = self.trades.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *trades)
    }

    /// 获取已记录的成交数量
    pub fn len(&self) -> usize {
        self.trades.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// 检查是否为空
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for TradeLog {
    fn default() -> Self {
        Self::new()
    }
}
