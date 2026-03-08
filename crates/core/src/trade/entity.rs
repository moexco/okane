use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// # Summary
/// 系统内的唯一账户标识，用于隔离不同用户的逻辑资金体系与策略归属。
///
/// # Invariants
/// - AccountId 在整个系统中必须全局唯一。
/// - 策略只与 AccountId 绑定，而不关心物理的 Broker 通道。
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct AccountId(pub String);

/// # Summary
/// 订单的系统内唯一标识。
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct OrderId(pub String);

/// # Summary
/// 订单的交易方向定义。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderDirection {
    /// 买入 (做多)
    Buy,
    /// 卖出 (做空)
    Sell,
}

/// # Summary
/// 订单的生命周期状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderStatus {
    /// 待处理 (刚创建，未提交到撮合引擎或物理券商)
    Pending,
    /// 已提交 (已发往券商，等待成交回报)
    Submitted,
    /// 部分成交
    PartialFilled,
    /// 完全成交
    Filled,
    /// 已撤销 (全部取消，尚未成交的部分被回收)
    Canceled,
    /// 拒绝 (风控拒绝或券商拒绝)
    Rejected,
}

/// # Summary
/// 详细的逻辑订单模型。这是策略生成的标准交易意图。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    /// 系统内全局唯一的订单 ID
    pub id: OrderId,
    /// 归属的逻辑系统账户
    pub account_id: AccountId,
    /// 交易标的
    pub symbol: String,
    /// 开平方向
    pub direction: OrderDirection,
    /// 委托价格。目前默认按市价成交时此值可能为空或作为限价。
    pub price: Option<Decimal>,
    /// 委托数量。（绝对值，例如买入 100 股）
    pub volume: Decimal,
    /// 已经成交的数量
    pub filled_volume: Decimal,
    /// 订单状态
    pub status: OrderStatus,
    /// 订单创建的系统时间戳 (毫秒)
    pub created_at: i64,
}

impl Order {
    /// # Logic
    /// 创建一笔全新的逻辑委托单，初始状态为 Pending。
    pub fn new(
        id: OrderId,
        account_id: AccountId,
        symbol: String,
        direction: OrderDirection,
        price: Option<Decimal>,
        volume: Decimal,
        now_ms: i64,
    ) -> Self {
        Self {
            id,
            account_id,
            symbol,
            direction,
            price,
            volume,
            filled_volume: Decimal::ZERO,
            status: OrderStatus::Pending,
            created_at: now_ms,
        }
    }
}

/// # Summary
/// 单笔撮合或券商的回报记录（流水/Trade）。
/// 用于精确计算资金变动、滑点和手续费。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    /// 关联的原始逻辑单 ID
    pub order_id: OrderId,
    /// 归属的逻辑系统账户
    pub account_id: AccountId,
    /// 交易标的
    pub symbol: String,
    /// 成交方向
    pub direction: OrderDirection,
    /// 实际成交价格
    pub price: Decimal,
    /// 实际成交数量
    pub volume: Decimal,
    /// 手续费 (按具体规则扣收)
    pub commission: Decimal,
    /// 成交时间戳 (毫秒)
    pub timestamp: i64,
}

/// # Summary
/// 指定标的的持仓记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    /// 归属账户
    pub account_id: AccountId,
    /// 资产标的
    pub symbol: String,
    /// 当前持有数量 (正数表示多头，负数表示空头)
    pub volume: Decimal,
    /// 持仓均价 (用于计算盈亏)
    pub average_price: Decimal,
}

impl Position {
    /// # Logic
    /// 初始化一个空持仓
    pub fn empty(account_id: AccountId, symbol: String) -> Self {
        Self {
            account_id,
            symbol,
            volume: Decimal::ZERO,
            average_price: Decimal::ZERO,
        }
    }
}

/// # Summary
/// 系统账户聚合根的数据快照。
/// 包含资金概况及全量持仓明细。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AccountSnapshot {
    pub account_id: AccountId,
    /// 账户可用资金余额（可用于开仓下单的现金）
    pub available_balance: Decimal,
    /// 冻结资金 (挂单中未成交部分占用的资金)
    pub frozen_balance: Decimal,
    /// 总权益 (可用 + 冻结 + 未结持仓盈亏)
    pub total_equity: Decimal,
    /// 持仓列表
    pub positions: Vec<Position>,
}

/// # Summary
/// 算法单类型定义。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "params")]
pub enum AlgoType {
    /// 网格交易：在价格区间内进行低买高卖。
    Grid {
        upper_price: Decimal,
        lower_price: Decimal,
        grids: u32,
    },
    /// 时间加权平均价格：在指定时间内均匀下单。
    Twap {
        duration_secs: u64,
        total_volume: Decimal,
    },
    /// 狙击单：极速交易，通常用于捕捉瞬时机会。
    Snipe {
        target_price: Decimal,
        max_slippage: Decimal,
    },
}

/// # Summary
/// 算法单状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlgoOrderStatus {
    /// 正在运行
    Running,
    /// 已暂停
    Paused,
    /// 已完成
    Completed,
    /// 已取消
    Canceled,
    /// 失败
    Failed,
}

/// # Summary
/// 算法单模型。
/// 算法单通常不直接对应物理订单，而是由算法逻辑驱动产生一笔或多笔子订单。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlgoOrder {
    /// 算法单唯一标识
    pub id: OrderId,
    /// 归属账户
    pub account_id: AccountId,
    /// 交易标的
    pub symbol: String,
    /// 算法类型及参数
    pub algo: AlgoType,
    /// 当前状态
    pub status: AlgoOrderStatus,
    /// 已成交总量
    pub filled_volume: Decimal,
    /// 创建时间
    pub created_at: i64,
}

impl AlgoOrder {
    pub fn new(
        id: OrderId,
        account_id: AccountId,
        symbol: String,
        algo: AlgoType,
        now_ms: i64,
    ) -> Self {
        Self {
            id,
            account_id,
            symbol,
            algo,
            status: AlgoOrderStatus::Running,
            filled_volume: Decimal::ZERO,
            created_at: now_ms,
        }
    }
}
