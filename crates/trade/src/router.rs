use async_trait::async_trait;
use okane_core::store::port::SystemStore;
use okane_core::trade::entity::{AccountId, AccountSnapshot, Order, OrderId};
use okane_core::trade::port::{TradeError, TradePort};
use std::sync::Arc;

/// # Summary
/// 基于逻辑交易账号后端的统一交易路由器。
///
/// 当前版本完整支持本地后端，并为平台后端保留显式的执行入口。
pub struct RoutedTradePort {
    local_trade_port: Arc<dyn TradePort>,
    system_store: Arc<dyn SystemStore>,
}

impl RoutedTradePort {
    pub fn new(local_trade_port: Arc<dyn TradePort>, system_store: Arc<dyn SystemStore>) -> Self {
        Self {
            local_trade_port,
            system_store,
        }
    }

    async fn type_for_account(&self, account_id: &str) -> Result<String, TradeError> {
        let profile = self
            .system_store
            .get_account_profile(account_id)
            .await
            .map_err(|e| TradeError::InternalError(format!("account profile lookup failed: {}", e)))?
            .ok_or_else(|| TradeError::AccountNotFound(account_id.to_string()))?;
        Ok(profile.account_type)
    }
}

#[async_trait]
impl TradePort for RoutedTradePort {
    async fn submit_order(&self, order: Order) -> Result<OrderId, TradeError> {
        match self.type_for_account(&order.account_id.0).await?.as_str() {
            "local" => self.local_trade_port.submit_order(order).await,
            account_type => Err(TradeError::BrokerIntegrationError(
                format!(
                    "account type {} is registered, but no platform gateway is configured",
                    account_type
                ),
            )),
        }
    }

    async fn cancel_order(&self, order_id: OrderId) -> Result<(), TradeError> {
        self.local_trade_port.cancel_order(order_id).await
    }

    async fn get_account(&self, account_id: AccountId) -> Result<AccountSnapshot, TradeError> {
        self.local_trade_port.get_account(account_id).await
    }

    async fn get_orders(&self, account_id: &AccountId) -> Result<Vec<Order>, TradeError> {
        self.local_trade_port.get_orders(account_id).await
    }

    async fn get_order(&self, order_id: &OrderId) -> Result<Option<Order>, TradeError> {
        self.local_trade_port.get_order(order_id).await
    }

    async fn ensure_account(
        &self,
        account_id: AccountId,
        initial_balance: rust_decimal::Decimal,
    ) -> Result<(), TradeError> {
        self.local_trade_port
            .ensure_account(account_id, initial_balance)
            .await
    }
}
