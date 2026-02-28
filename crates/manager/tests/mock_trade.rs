use async_trait::async_trait;
use okane_core::trade::entity::{AccountId, AccountSnapshot, Order, OrderId};
use okane_core::trade::port::{TradeError, TradePort};

pub struct MockTradePort;

#[async_trait]
impl TradePort for MockTradePort {
    async fn submit_order(&self, _order: Order) -> Result<OrderId, TradeError> {
        Ok(OrderId("mock-order-id".to_string()))
    }

    async fn cancel_order(&self, _order_id: OrderId) -> Result<(), TradeError> {
        Ok(())
    }

    async fn get_account(&self, _account_id: AccountId) -> Result<AccountSnapshot, TradeError> {
        unimplemented!()
    }
}
