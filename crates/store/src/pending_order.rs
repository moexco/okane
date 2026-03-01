use async_trait::async_trait;
use okane_core::trade::entity::{AccountId, Order, OrderId, OrderStatus};
use okane_core::trade::port::{PendingOrderPort, TradeError};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// # Summary
/// 基于内存的处于存活状态的系统内未完结活动的订单仓储实现。
///
/// 作为 `PendingOrderPort` 的适配器，提供对暂存单的管理能力。
pub struct MemoryPendingOrderStore {
    orders: Arc<RwLock<HashMap<OrderId, Order>>>,
}

impl MemoryPendingOrderStore {
    pub fn new() -> Self {
        Self {
            orders: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for MemoryPendingOrderStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PendingOrderPort for MemoryPendingOrderStore {
    async fn save(&self, order: Order) -> Result<(), TradeError> {
        self.orders.write().await.insert(order.id.clone(), order);
        Ok(())
    }

    async fn remove(&self, order_id: &OrderId) -> Result<Option<Order>, TradeError> {
        Ok(self.orders.write().await.remove(order_id))
    }

    async fn get(&self, order_id: &OrderId) -> Result<Option<Order>, TradeError> {
        Ok(self.orders.read().await.get(order_id).cloned())
    }

    async fn get_by_account(&self, account_id: &AccountId) -> Result<Vec<Order>, TradeError> {
        let guard = self.orders.read().await;
        Ok(guard.values().filter(|o| o.account_id == *account_id).cloned().collect())
    }

    async fn get_by_symbol(&self, symbol: &str) -> Result<Vec<Order>, TradeError> {
        let guard = self.orders.read().await;
        Ok(guard.values().filter(|o| o.symbol == symbol).cloned().collect())
    }

    async fn update_status(&self, order_id: &OrderId, status: OrderStatus) -> Result<(), TradeError> {
        let mut guard = self.orders.write().await;
        if let Some(order) = guard.get_mut(order_id) {
            order.status = status;
        }
        Ok(())
    }
}
