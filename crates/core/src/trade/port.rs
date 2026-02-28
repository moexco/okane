use super::entity::{AccountId, AccountSnapshot, Order, OrderId, Trade};
use async_trait::async_trait;
use crate::market::entity::Candle;
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

/// # Summary
/// 针对回测环境扩展的方法，允许时间驱动器推进撮合进度。
#[async_trait]
pub trait BacktestTradePort: TradePort {
    /// 回测驱动器推进一根 K 线时调用，用于撮合挂在账本中的委托单。
    async fn tick(&self, symbol: &str, candle: &Candle) -> Result<(), TradeError>;
}

/// # Summary
/// 针对账户资产的管理服务端口 (Repository / Port)。
#[async_trait]
pub trait AccountPort: Send + Sync {
    /// 开仓挂单时，请求冻结预估金额。
    async fn freeze_funds(&self, account_id: &AccountId, amount: rust_decimal::Decimal) -> Result<(), TradeError>;

    /// 撤单时解冻未使用的金额。
    async fn unfreeze_funds(&self, account_id: &AccountId, amount: rust_decimal::Decimal) -> Result<(), TradeError>;

    /// 行情撮合成功后，交由账户中心进行原子化持仓更新与资金结算。
    async fn process_trade(&self, account_id: &AccountId, trade: &Trade, est_req_funds: rust_decimal::Decimal) -> Result<(), TradeError>;

    /// 快照截取
    async fn snapshot(&self, account_id: &AccountId) -> Result<AccountSnapshot, TradeError>;
}

/// # Summary
/// 本地或远程撮合引擎对接端口。
pub trait MatcherPort: Send + Sync {
    /// 执行/评估一张尚未完结的订单 (市场价或现价单，与当前 K 线价位对比)。
    fn execute_order(&self, order: &mut Order, current_price: rust_decimal::Decimal, timestamp: i64) -> Option<Trade>;
}

/// # Summary
/// 物理券商网关对接端口 (Broker Gateway Port)。
/// 用于定义与外部真实交易所 (IB, Futu, Binance 等) 进行交互的标准接口。
/// 未来的实盘组件将实现此接口。
#[async_trait]
pub trait BrokerPort: Send + Sync {
    /// 向外部网关发送一笔真实物理订单
    async fn send_order(&self, order: &Order) -> Result<String, TradeError>;
    
    /// 向外部网关请求取消某笔尚未成交的单子
    async fn cancel_order(&self, external_order_id: &str) -> Result<(), TradeError>;
    
    /// 主动同步/查询某笔外发订单的最新网关状态 (如是否部分成交)
    async fn query_order_status(&self, external_order_id: &str) -> Result<(), TradeError>;
    
    // TODO: 未来还会增加类似 subscribe_execution_report 的流式回报接口
}
