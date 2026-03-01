//! # DTO (Data Transfer Object) 层
//!
//! 将内部领域模型转化为面向前端 JSON 输出的轻量结构体。
//! 所有 DTO 必须派生 `utoipa::ToSchema` 以自动进入 Swagger 文档。

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// ============================================================
//  账户相关 DTO
// ============================================================

/// 持仓明细 DTO
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PositionResponse {
    /// 资产标的代码
    #[schema(example = "AAPL")]
    pub symbol: String,
    /// 持仓数量 (正=多头, 负=空头)
    #[schema(example = "100.00")]
    pub volume: String,
    /// 持仓均价
    #[schema(example = "175.50")]
    pub average_price: String,
}

/// 账户快照 DTO - 对应 UI 顶部 Key Metrics 区域
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AccountSnapshotResponse {
    /// 系统账户 ID
    #[schema(example = "SysAcct_Alpha_01")]
    pub account_id: String,
    /// 可用资金余额
    #[schema(example = "312980.50")]
    pub available_balance: String,
    /// 冻结资金 (挂单中)
    #[schema(example = "15000.00")]
    pub frozen_balance: String,
    /// 总权益
    #[schema(example = "1245670.32")]
    pub total_equity: String,
    /// 当前持仓列表
    pub positions: Vec<PositionResponse>,
}

/// 订单流 DTO
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OrderResponse {
    /// 订单 ID
    #[schema(example = "ord-123456")]
    pub id: String,
    /// 归属账户 ID
    #[schema(example = "SysAcct_Alpha_01")]
    pub account_id: String,
    /// 股票代码
    #[schema(example = "NVDA")]
    pub symbol: String,
    /// 方向 (Buy/Sell)
    #[schema(example = "Buy")]
    pub direction: String,
    /// 限价 (市价单为 null)
    #[schema(example = "120.50")]
    pub price: Option<String>,
    /// 委托数量
    #[schema(example = "100")]
    pub volume: String,
    /// 已成交数量
    #[schema(example = "50")]
    pub filled_volume: String,
    /// 状态 (Pending, Filled, Canceled 等)
    #[schema(example = "Pending")]
    pub status: String,
    /// 创建时间 (毫秒级时间戳)
    #[schema(example = 1710000000000_i64)]
    pub created_at: i64,
}

// ============================================================
//  行情相关 DTO
// ============================================================

/// 股票元数据 DTO
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StockMetadataResponse {
    /// 股票代码
    #[schema(example = "AAPL")]
    pub symbol: String,
    /// 股票名称
    #[schema(example = "Apple Inc.")]
    pub name: String,
    /// 交易所
    #[schema(example = "NASDAQ")]
    pub exchange: String,
    /// 货币
    #[schema(example = "USD")]
    pub currency: String,
    /// 板块
    #[schema(example = "Technology")]
    pub sector: Option<String>,
}

/// K 线数据 DTO
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CandleResponse {
    /// 时间戳 (ISO 8601)
    #[schema(example = "2026-03-01T10:00:00Z")]
    pub time: String,
    /// 开盘价
    #[schema(example = "150.5")]
    pub open: String,
    /// 最高价
    #[schema(example = "152.0")]
    pub high: String,
    /// 最低价
    #[schema(example = "149.0")]
    pub low: String,
    /// 收盘价
    #[schema(example = "151.0")]
    pub close: String,
    /// 成交量
    #[schema(example = "1000000")]
    pub volume: String,
    /// 是否已完结
    #[schema(example = true)]
    pub is_final: bool,
}

// ============================================================
//  策略相关 DTO
// ============================================================

/// 策略实例 DTO
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StrategyResponse {
    /// 策略实例 ID
    #[schema(example = "a1b2c3d4-e5f6-7890")]
    pub id: String,
    /// 交易标的
    #[schema(example = "NVDA")]
    pub symbol: String,
    /// 绑定的系统账户
    #[schema(example = "SysAcct_Alpha_01")]
    pub account_id: String,
    /// K 线周期 (如 "1m", "5m", "1d")
    #[schema(example = "5m")]
    pub timeframe: String,
    /// 引擎类型 (JavaScript / Wasm)
    #[schema(example = "JavaScript")]
    pub engine_type: String,
    /// 当前状态 (Pending / Running / Stopped / Failed)
    #[schema(example = "Running")]
    pub status: String,
    /// 创建时间 (ISO 8601)
    #[schema(example = "2026-03-01T00:00:00Z")]
    pub created_at: String,
}

/// 启动策略请求体 DTO
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StartStrategyRequest {
    /// 目标证券代码
    #[schema(example = "AAPL")]
    pub symbol: String,
    /// 绑定的系统账户 ID
    #[schema(example = "SysAcct_Alpha_01")]
    pub account_id: String,
    /// K 线时间周期
    #[schema(example = "5m")]
    pub timeframe: String,
    /// 引擎类型 ("JavaScript" 或 "Wasm")
    #[schema(example = "JavaScript")]
    pub engine_type: String,
    /// 策略源码 (base64 编码的脚本)
    #[schema(example = "Y29uc29sZS5sb2coJ2hlbGxvJyk7")]
    pub source_base64: String,
}

/// 保存策略源码请求体 DTO
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct SaveStrategySourceRequest {
    /// 策略源码 (base64 编码)
    #[schema(example = "Y29uc29sZS5sb2coJ2hlbGxvJyk7")]
    pub source_base64: String,
}

// ============================================================
//  通用响应 DTO
// ============================================================

/// 统一 API 响应包装器
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApiResponse<T: Serialize + ToSchema> {
    /// 是否成功
    pub success: bool,
    /// 数据载荷 (成功时)
    pub data: Option<T>,
    /// 错误信息 (失败时)
    pub error: Option<String>,
}

impl<T: Serialize + ToSchema> ApiResponse<T> {
    /// 构建成功响应
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }
}

/// 构建失败响应 (不含泛型载荷)
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ApiErrorResponse {
    /// 固定为 false
    pub success: bool,
    /// 错误描述信息
    pub error: String,
}

impl ApiErrorResponse {
    /// 从错误信息构建
    pub fn from_msg(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            error: msg.into(),
        }
    }
}

// ============================================================
//  鉴权 DTO
// ============================================================

/// 登录请求体
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LoginRequest {
    /// 用户名
    #[schema(example = "admin")]
    pub username: String,
    /// 密码
    #[schema(example = "password123")]
    pub password: String,
}

/// 修改密码请求体
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChangePasswordRequest {
    /// 原密码
    #[schema(example = "oldpassword123")]
    pub old_password: String,
    /// 新密码
    #[schema(example = "newSecurePwd!")]
    pub new_password: String,
}

/// 创建新用户请求体 (仅管理员)
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateUserRequest {
    /// 用户登录 ID
    #[schema(example = "trader_01")]
    pub id: String,
    /// 用户显示名
    #[schema(example = "John Doe")]
    pub name: String,
    /// 新用户密码
    #[schema(example = "P@ssw0rd!")]
    pub password: String,
    /// 角色 (Admin 或 Standard)
    #[schema(example = "Standard")]
    pub role: String,
}

/// 用户基础信息响应 DTO
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UserResponse {
    /// 用户唯一标识
    #[schema(example = "admin")]
    pub id: String,
    /// 显示名称
    #[schema(example = "System Administrator")]
    pub name: String,
    /// 角色
    #[schema(example = "Admin")]
    pub role: String,
    /// 注册时间
    #[schema(example = "2026-03-01T00:00:00Z")]
    pub created_at: String,
}

/// 登录成功返回的 Token
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LoginResponse {
    /// JWT Bearer Token
    #[schema(example = "eyJhbGciOiJIUzI1NiIs...")]
    pub token: String,
    /// Token 过期时间 (秒)
    #[schema(example = 86400)]
    pub expires_in: u64,
}

/// JWT Claims 内容 (内部使用，不暴露到 Swagger)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// 用户唯一标识
    pub sub: String,
    /// 角色 ("user" 或 "admin")
    pub role: String,
    /// Token 过期时间 (Unix 时间戳)
    pub exp: usize,
}

// ============================================================
//  领域模型 → DTO 惯用转换 (impl From<T>)
// ============================================================

impl From<okane_core::trade::entity::Position> for PositionResponse {
    fn from(p: okane_core::trade::entity::Position) -> Self {
        Self {
            symbol: p.symbol,
            volume: p.volume.to_string(),
            average_price: p.average_price.to_string(),
        }
    }
}

impl From<okane_core::trade::entity::AccountSnapshot> for AccountSnapshotResponse {
    fn from(s: okane_core::trade::entity::AccountSnapshot) -> Self {
        Self {
            account_id: s.account_id.0,
            available_balance: s.available_balance.to_string(),
            frozen_balance: s.frozen_balance.to_string(),
            total_equity: s.total_equity.to_string(),
            positions: s.positions.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<&okane_core::strategy::entity::StrategyInstance> for StrategyResponse {
    fn from(i: &okane_core::strategy::entity::StrategyInstance) -> Self {
        Self {
            id: i.id.clone(),
            symbol: i.symbol.clone(),
            account_id: i.account_id.clone(),
            timeframe: format!("{}", i.timeframe),
            engine_type: format!("{}", i.engine_type),
            status: format!("{:?}", i.status),
            created_at: i.created_at.to_rfc3339(),
        }
    }
}

impl From<&okane_core::store::port::User> for UserResponse {
    fn from(u: &okane_core::store::port::User) -> Self {
        Self {
            id: u.id.clone(),
            name: u.name.clone(),
            role: u.role.to_string(),
            created_at: u.created_at.to_rfc3339(),
        }
    }
}

impl From<okane_core::store::port::StockMetadata> for StockMetadataResponse {
    fn from(m: okane_core::store::port::StockMetadata) -> Self {
        Self {
            symbol: m.symbol,
            name: m.name,
            exchange: m.exchange,
            currency: m.currency,
            sector: m.sector,
        }
    }
}

impl From<okane_core::market::entity::Candle> for CandleResponse {
    fn from(c: okane_core::market::entity::Candle) -> Self {
        Self {
            time: c.time.to_rfc3339(),
            open: Decimal::from_f64_retain(c.open).unwrap_or(Decimal::ZERO).to_string(),
            high: Decimal::from_f64_retain(c.high).unwrap_or(Decimal::ZERO).to_string(),
            low: Decimal::from_f64_retain(c.low).unwrap_or(Decimal::ZERO).to_string(),
            close: Decimal::from_f64_retain(c.close).unwrap_or(Decimal::ZERO).to_string(),
            volume: Decimal::from_f64_retain(c.volume).unwrap_or(Decimal::ZERO).to_string(),
            is_final: c.is_final,
        }
    }
}

impl From<okane_core::trade::entity::Order> for OrderResponse {
    fn from(o: okane_core::trade::entity::Order) -> Self {
        Self {
            id: o.id.0,
            account_id: o.account_id.0,
            symbol: o.symbol,
            direction: format!("{:?}", o.direction),
            price: o.price.map(|p| p.to_string()),
            volume: o.volume.to_string(),
            filled_volume: o.filled_volume.to_string(),
            status: format!("{:?}", o.status),
            created_at: o.created_at,
        }
    }
}
