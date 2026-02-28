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
    #[schema(value_type = String, example = "100")]
    pub volume: Decimal,
    /// 持仓均价
    #[schema(value_type = String, example = "175.50")]
    pub average_price: Decimal,
}

/// 账户快照 DTO - 对应 UI 顶部 Key Metrics 区域
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AccountSnapshotResponse {
    /// 系统账户 ID
    #[schema(example = "SysAcct_Alpha_01")]
    pub account_id: String,
    /// 可用资金余额
    #[schema(value_type = String, example = "312980.50")]
    pub available_balance: Decimal,
    /// 冻结资金 (挂单中)
    #[schema(value_type = String, example = "15000.00")]
    pub frozen_balance: Decimal,
    /// 总权益
    #[schema(value_type = String, example = "1245670.32")]
    pub total_equity: Decimal,
    /// 当前持仓列表
    pub positions: Vec<PositionResponse>,
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
            volume: p.volume,
            average_price: p.average_price,
        }
    }
}

impl From<okane_core::trade::entity::AccountSnapshot> for AccountSnapshotResponse {
    fn from(s: okane_core::trade::entity::AccountSnapshot) -> Self {
        Self {
            account_id: s.account_id.0,
            available_balance: s.available_balance,
            frozen_balance: s.frozen_balance,
            total_equity: s.total_equity,
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
