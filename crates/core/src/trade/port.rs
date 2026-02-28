use super::entity::{AccountId, OrderId, Order, AccountSnapshot};
use async_trait::async_trait;
use thiserror::Error;

/// # Summary
/// 交易执行环节中可能发生的错误。
#[derive(Error, Debug)]
pub enum TradeError {
    #[error("账户不存在: {0}")]
    AccountNotFound(String),
    #[error("可用资金不足. 需要: {required}, 实际: {actual}")]
    InsufficientFunds {
        required: rust_decimal::Decimal,
        actual: rust_decimal::Decimal,
    },
    #[error("订单未找到或不存在: {0}")]
    OrderNotFound(String),
    #[error("订单状态不允许该操作")]
    InvalidOrderStatus,
    #[error("底层券商通道错误: {0}")]
    BrokerIntegrationError(String),
    #[error("内部系统错误: {0}")]
    InternalError(String),
}

/// # Summary
/// 核心交易服务抽象接口。
/// 策略引擎和前端调度通过此端口对逻辑账户下发订单意图和查询快照，
/// 它是业务逻辑向底层基础设施（撮合引擎或物理券商API）发送请求的唯一门户。
///
/// # Invariants
/// - 此接口必须是异步且线程安全的 (`Send + Sync`)，因其可能涉及并发的跨服务路由。
#[async_trait]
pub trait TradePort: Send + Sync {
    /// 提交一笔新的逻辑单
    ///
    /// # Arguments
    /// * `order` - 由策略产生的标准化逻辑订单信息
    ///
    /// # Returns
    /// * `Ok(OrderId)` - 订单进入 pending/submitted 状态的唯一追踪 ID
    /// * `Err(TradeError)` - 如果资金不足、风控拦截或路由失败
    async fn submit_order(&self, order: Order) -> Result<OrderId, TradeError>;

    /// 撤销一笔尚未完全成交的委托单
    ///
    /// # Arguments
    /// * `order_id` - 订单系统 ID
    async fn cancel_order(&self, order_id: OrderId) -> Result<(), TradeError>;

    /// 查询某个逻辑账户的当前资金和持仓快照
    ///
    /// # Arguments
    /// * `account_id` - 待查询的系统级 Account ID
    async fn get_account(&self, account_id: AccountId) -> Result<AccountSnapshot, TradeError>;
}
