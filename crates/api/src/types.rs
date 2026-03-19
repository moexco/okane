//! # DTO (Data Transfer Object) 层
//!
//! 将内部领域模型转化为面向前端 JSON 输出的轻量结构体。
//! 所有 DTO 必须派生 `utoipa::ToSchema` 以自动进入 Swagger 文档。

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
    /// 逻辑交易账号 ID
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

/// 逻辑交易账号档案 DTO
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AccountProfileResponse {
    /// 逻辑交易账号 ID
    #[schema(example = "acct_3f2c3af9b7aa4e56")]
    pub account_id: String,
    /// 用户定义的账号名称
    #[schema(example = "我的本地测试账户")]
    pub account_name: String,
    /// 账号类型
    #[schema(example = "local")]
    #[serde(rename = "type")]
    pub account_type: String,
    /// 账号配置
    pub config: serde_json::Value,
    /// 创建时间
    #[schema(example = "2026-03-19T10:00:00Z")]
    pub created_at: String,
}

/// 创建新逻辑交易账号请求体
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateAccountRequest {
    /// 用户定义的账号名称
    #[schema(example = "我的 Schwab 主账户")]
    pub account_name: String,
    /// 账号类型，例如 `local`、`futu`、`schwab`。
    #[schema(example = "local")]
    #[serde(rename = "type")]
    pub account_type: String,
    /// 账号配置。不同账号类型由对应实现解释。
    pub config: serde_json::Value,
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

/// 算法单 DTO
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AlgoOrderResponse {
    /// 算法单 ID
    #[schema(example = "algo-123456")]
    pub id: String,
    /// 归属账户 ID
    #[schema(example = "SysAcct_Alpha_01")]
    pub account_id: String,
    /// 股票代码
    #[schema(example = "NVDA")]
    pub symbol: String,
    /// 算法类型 (当前对外支持 Snipe)
    #[schema(example = "Snipe")]
    pub algo_type: String,
    /// 算法参数 (JSON 对象)
    pub params: serde_json::Value,
    /// 状态 (Running, Completed, Canceled 等)
    #[schema(example = "Running")]
    pub status: String,
    /// 目标数量
    pub requested_volume: String,
    /// 已成交数量
    pub filled_volume: String,
    /// 创建时间
    pub created_at: i64,
}

/// 历史成交明细 (Trade/Fill) DTO
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TradeResponse {
    /// 归属账户 ID
    #[schema(example = "SysAcct_Alpha_01")]
    pub account_id: String,
    /// 原始订单 ID
    #[schema(example = "ord-123456")]
    pub order_id: String,
    /// 股票代码
    #[schema(example = "NVDA")]
    pub symbol: String,
    /// 方向 (Buy/Sell)
    #[schema(example = "Buy")]
    pub direction: String,
    /// 实际成交价
    #[schema(example = "120.50")]
    pub price: String,
    /// 实际成交量
    #[schema(example = "50")]
    pub volume: String,
    /// 手续费
    #[schema(example = "0.5")]
    pub commission: String,
    /// 成交时间戳 (毫秒)
    #[schema(example = 1710000000000_i64)]
    pub timestamp: i64,
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
    /// 策略名称
    #[schema(example = "NVDA Momentum")]
    pub name: String,
    /// 交易标的
    #[schema(example = "NVDA")]
    pub symbol: String,
    /// 默认挂载的逻辑交易账号
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
    /// 当前参数定义
    pub parameter_schema: serde_json::Value,
    /// 最新运行记录 ID
    #[schema(example = "run-b33f5d48")]
    pub latest_run_id: Option<String>,
    /// 策略源码 (Base64 编码)
    #[schema(example = "Y29uc29sZS5sb2coJ2hlbGxvJyk7")]
    pub source_base64: String,
    /// 创建时间 (ISO 8601)
    #[schema(example = "2026-03-01T00:00:00Z")]
    pub created_at: String,
    /// 更新时间 (ISO 8601)
    #[schema(example = "2026-03-02T00:00:00Z")]
    pub updated_at: String,
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
    /// 运行模式
    #[schema(example = "LivePaper", default = "LivePaper")]
    pub run_mode: Option<String>,
    /// 策略源码 (base64 编码的脚本)
    #[schema(example = "Y29uc29sZS5sb2coJ2hlbGxvJyk7")]
    pub source_base64: String,
}

/// 保存策略源码请求体 DTO
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SaveStrategySourceRequest {
    /// 策略源码 (base64 编码)
    #[schema(example = "Y29uc29sZS5sb2coJ2hlbGxvJyk7")]
    pub source_base64: String,
}

// ============================================================
//  通知配置 Request/Response
// ============================================================

/// Telegram 推送配置
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TelegramConfig {
    /// Telegram Bot API Token
    #[schema(example = "123456789:ABCdefGHIjklMNOpqrSTUvwxYZ")]
    pub bot_token: String,
    /// 目标 Chat ID
    #[schema(example = "-1001234567890")]
    pub chat_id: String,
}

/// Email 推送配置
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EmailConfig {
    /// SMTP 服务器地址
    #[schema(example = "smtp.example.com")]
    pub smtp_host: String,
    /// SMTP 用户名
    #[schema(example = "system@example.com")]
    pub smtp_user: String,
    /// SMTP 密码
    #[schema(example = "s3cr3tP4ssw0rd")]
    pub smtp_pass: String,
    /// 发件人
    #[schema(example = "system@example.com")]
    pub from: String,
    /// 收件人
    #[schema(example = "user@example.com")]
    pub to: String,
}

/// 用户级通知配置请求体
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateNotifyConfigRequest {
    /// 通知渠道: "none" | "telegram" | "email"
    #[schema(example = "telegram")]
    pub channel: String,
    /// Telegram 推送配置
    pub telegram: TelegramConfig,
    /// Email 推送配置
    pub email: EmailConfig,
}

/// 用户级通知配置响应体
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NotifyConfigResponse {
    /// 通知渠道: "none" | "telegram" | "email"
    #[schema(example = "telegram")]
    pub channel: String,
    /// Telegram 推送配置
    pub telegram: TelegramConfig,
    /// Email 推送配置
    pub email: EmailConfig,
}

// ============================================================
//  回测相关 DTO
// ============================================================

/// 执行回测请求体
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BacktestRequest {
    /// 目标证券代码
    #[schema(example = "AAPL")]
    pub symbol: String,
    /// K 线时间周期
    #[schema(example = "5m")]
    pub timeframe: String,
    /// 开始时间 (RFC3339 格式)
    #[schema(example = "2026-01-01T00:00:00Z")]
    pub start: String,
    /// 结束时间 (RFC3339 格式)
    #[schema(example = "2026-02-01T00:00:00Z")]
    pub end: String,
    /// 初始资金
    #[schema(example = "100000.00")]
    pub initial_balance: String,
    /// 引擎类型 ("JavaScript" 或 "Wasm")
    #[schema(example = "JavaScript")]
    pub engine_type: String,
    /// 策略源码 (base64 编码的脚本)
    #[schema(example = "Y29uc29sZS5sb2coJ2hlbGxvJyk7")]
    pub source_base64: String,
}

/// 回测结果
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BacktestResponse {
    /// 回测结束时的账户快照
    pub final_snapshot: AccountSnapshotResponse,
    /// 完整交易流水
    pub trades: Vec<TradeResponse>,
    /// 共处理的 K 线数量
    #[schema(example = 5432)]
    pub candle_count: usize,
}

/// 分页数据包装器
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Page<T: Serialize + ToSchema> {
    /// 数据列表
    pub items: Vec<T>,
    /// 总记录数
    #[schema(example = 100)]
    pub total: usize,
    /// 跳过的记录数
    #[schema(example = 0)]
    pub offset: usize,
    /// 分页大小
    #[schema(example = 50)]
    pub limit: usize,
}

impl<T: Serialize + ToSchema> Page<T> {
    /// 创建分页对象
    pub fn new(items: Vec<T>, total: usize, offset: usize, limit: usize) -> Self {
        Self {
            items,
            total,
            offset,
            limit,
        }
    }
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
    /// 接口处理耗时 (毫秒)
    pub latency_ms: Option<u64>,
    #[schema(example = 200)]
    pub code: u32,
}

impl<T: Serialize + ToSchema> ApiResponse<T> {
    /// 构建成功响应 (旧版兼容)
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
            code: 200,
            latency_ms: None,
        }
    }
}

/// 构建失败响应 (不含泛型载荷)
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApiErrorResponse {
    /// 固定为 false
    pub success: bool,
    /// 错误描述信息
    pub error: String,
    /// 接口处理耗时 (毫秒)
    pub latency_ms: Option<u64>,
    /// 业务错误码 (5 位数)
    pub code: u32,
}

impl ApiErrorResponse {
    /// 从错误信息和状态码构建
    pub fn new(msg: impl Into<String>, code: u32) -> Self {
        Self {
            success: false,
            error: msg.into(),
            code,
            latency_ms: None,
        }
    }

    /// 从错误信息构建 (默认使用系统错误码 00999)
    pub fn from_msg(msg: impl Into<String>) -> Self {
        Self::new(msg, 999)
    }
}

// ============================================================
//  扩展：用于中间件单次序列化的机制
// ============================================================

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// 被擦除类型的响应数据，由中间件负责序列化
pub trait ErasedResponse: Send + Sync + 'static {
    fn status(&self) -> StatusCode;
    fn code(&self) -> u32;
    fn latency_ms(&self) -> Option<u64>;
    fn render(&self, latency: u64) -> Vec<u8>;
}

fn render_serialization_failure(code: u32, latency: u64, err: &serde_json::Error) -> Vec<u8> {
    let safe_error = ApiErrorResponse {
        success: false,
        error: format!("response serialization failure: {}", err),
        code,
        latency_ms: Some(latency),
    };
    serde_json::to_vec(&safe_error).unwrap_or_else(|fallback_err| {
        tracing::error!(
            "failed to serialize fallback error response: {}",
            fallback_err
        );
        br#"{"success":false,"error":"response serialization failure: fallback serialization failed","code":500,"latency_ms":null}"#.to_vec()
    })
}

/// 成功响应的中间态
pub struct SuccessMarker<T: Serialize + ToSchema + Send + Sync + 'static> {
    pub data: T,
    pub status: StatusCode,
    pub code: u32,
    pub latency_ms: Option<u64>,
}

impl<T: Serialize + ToSchema + Send + Sync + 'static> ErasedResponse for SuccessMarker<T> {
    fn status(&self) -> StatusCode {
        self.status
    }

    fn code(&self) -> u32 {
        self.code
    }

    fn latency_ms(&self) -> Option<u64> {
        self.latency_ms
    }

    fn render(&self, latency: u64) -> Vec<u8> {
        // 使用临时结构体进行序列化，支持引用数据
        #[derive(Serialize)]
        struct TempResponse<'a, D: Serialize> {
            success: bool,
            data: Option<&'a D>,
            error: Option<String>,
            code: u32,
            latency_ms: Option<u64>,
        }

        let resp = TempResponse {
            success: true,
            data: Some(&self.data),
            error: None,
            code: self.code,
            latency_ms: Some(latency),
        };
        match serde_json::to_vec(&resp) {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::error!("failed to serialize success response: {}", err);
                render_serialization_failure(500, latency, &err)
            }
        }
    }
}

/// 错误响应的中间态
pub struct ErrorMarker {
    pub message: String,
    pub status: StatusCode,
    pub code: u32,
    pub latency_ms: Option<u64>,
}

impl ErasedResponse for ErrorMarker {
    fn status(&self) -> StatusCode {
        self.status
    }

    fn code(&self) -> u32 {
        self.code
    }

    fn latency_ms(&self) -> Option<u64> {
        self.latency_ms
    }

    fn render(&self, latency: u64) -> Vec<u8> {
        let resp = ApiErrorResponse {
            success: false,
            error: self.message.clone(),
            code: self.code,
            latency_ms: Some(latency),
        };
        match serde_json::to_vec(&resp) {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::error!("failed to serialize error response: {}", err);
                render_serialization_failure(self.code, latency, &err)
            }
        }
    }
}

/// Handler 返回的统一结果包装器
pub struct ApiResult<T>(pub T);

use std::sync::Arc;

impl<T: Serialize + ToSchema + Send + Sync + 'static> IntoResponse for ApiResult<T> {
    fn into_response(self) -> Response {
        let mut res = StatusCode::OK.into_response();
        let marker = SuccessMarker {
            data: self.0,
            status: StatusCode::OK,
            code: 200,
            latency_ms: None,
        };
        res.extensions_mut()
            .insert(Arc::new(marker) as Arc<dyn ErasedResponse>);
        res
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
    /// 客户端唯一标识 (用于实现单设备一个 Session 复用)
    #[schema(example = "browser_chrome_1")]
    pub client_id: String,
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
    /// JWT Access Token (短效)
    #[schema(example = "eyJhbGciOiJIUzI1NiIs...")]
    pub access_token: String,
    /// JWT Refresh Token (长效)
    #[schema(example = "eyJhbGciOiJIUzI1NiIs...")]
    pub refresh_token: String,
    /// Access Token 过期时间 (秒)
    #[schema(example = 900)]
    pub expires_in: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateSettingsRequest {
    pub setting_key: String,
    pub setting_value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WatchlistRequest {
    pub symbol: String,
}

/// JWT Claims 内容 (内部使用，不暴露到 Swagger)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// 用户唯一标识 (user_id)
    pub sub: String,
    /// 会话唯一标识 (session_id)
    pub sid: String,
    /// 令牌唯一标识 (jti)
    pub jti: String,
    /// 角色 ("Admin" 或 "Standard")
    pub role: String,
    /// 是否需要强制改密
    pub force_password_change: bool,
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

impl From<okane_core::store::port::AccountProfile> for AccountProfileResponse {
    fn from(profile: okane_core::store::port::AccountProfile) -> Self {
        Self {
            account_id: profile.id,
            account_name: profile.account_name,
            account_type: profile.account_type,
            config: profile.config,
            created_at: profile.created_at.to_rfc3339(),
        }
    }
}

impl From<&okane_core::strategy::entity::StrategyInstance> for StrategyResponse {
    fn from(i: &okane_core::strategy::entity::StrategyInstance) -> Self {
        use base64::Engine;
        Self {
            id: i.id.clone(),
            name: i.name.clone(),
            symbol: i.symbol.clone(),
            account_id: i.account_id.clone(),
            timeframe: format!("{}", i.timeframe),
            engine_type: format!("{}", i.engine_type),
            status: i.status.to_string(),
            parameter_schema: i.parameter_schema.clone(),
            latest_run_id: i.latest_run_id.clone(),
            source_base64: base64::prelude::BASE64_STANDARD.encode(&i.source),
            created_at: i.created_at.to_rfc3339(),
            updated_at: i.updated_at.to_rfc3339(),
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
            open: c.open.to_string(),
            high: c.high.to_string(),
            low: c.low.to_string(),
            close: c.close.to_string(),
            volume: c.volume.to_string(),
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

impl From<okane_core::trade::entity::Trade> for TradeResponse {
    fn from(t: okane_core::trade::entity::Trade) -> Self {
        Self {
            account_id: t.account_id.0,
            order_id: t.order_id.0,
            symbol: t.symbol,
            direction: format!("{:?}", t.direction),
            price: t.price.to_string(),
            volume: t.volume.to_string(),
            commission: t.commission.to_string(),
            timestamp: t.timestamp,
        }
    }
}

impl TryFrom<okane_core::trade::entity::AlgoOrder> for AlgoOrderResponse {
    type Error = String;

    fn try_from(o: okane_core::trade::entity::AlgoOrder) -> Result<Self, Self::Error> {
        let val = serde_json::to_value(&o.algo)
            .map_err(|err| format!("failed to serialize algo order params: {}", err))?;
        let algo_type = val
            .get("type")
            .and_then(|t| t.as_str())
            .map(ToOwned::to_owned)
            .ok_or_else(|| "failed to extract algo type from serialized params".to_string())?;
        let params = val
            .get("params")
            .cloned()
            .ok_or_else(|| "failed to extract algo params from serialized params".to_string())?;

        Ok(Self {
            id: o.id.0,
            account_id: o.account_id.0,
            symbol: o.symbol,
            algo_type,
            params,
            status: format!("{:?}", o.status),
            requested_volume: o.requested_volume.to_string(),
            filled_volume: o.filled_volume.to_string(),
            created_at: o.created_at,
        })
    }
}

impl From<okane_core::config::UserNotifyConfig> for NotifyConfigResponse {
    fn from(c: okane_core::config::UserNotifyConfig) -> Self {
        Self {
            channel: c.channel,
            telegram: TelegramConfig {
                bot_token: c.telegram.bot_token,
                chat_id: c.telegram.chat_id,
            },
            email: EmailConfig {
                smtp_host: c.email.smtp_host,
                smtp_user: c.email.smtp_user,
                smtp_pass: c.email.smtp_pass,
                from: c.email.from,
                to: c.email.to,
            },
        }
    }
}

impl From<UpdateNotifyConfigRequest> for okane_core::config::UserNotifyConfig {
    fn from(dto: UpdateNotifyConfigRequest) -> Self {
        Self {
            channel: dto.channel,
            telegram: okane_core::config::TelegramConfig {
                bot_token: dto.telegram.bot_token,
                chat_id: dto.telegram.chat_id,
            },
            email: okane_core::config::EmailConfig {
                smtp_host: dto.email.smtp_host,
                smtp_user: dto.email.smtp_user,
                smtp_pass: dto.email.smtp_pass,
                from: dto.email.from,
                to: dto.email.to,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::render_serialization_failure;

    #[test]
    fn test_render_serialization_failure_keeps_error_context()
    -> Result<(), Box<dyn std::error::Error>> {
        let err = match serde_json::from_str::<serde_json::Value>("{") {
            Ok(_) => return Err("expected parse err".into()),
            Err(err) => err,
        };
        let bytes = render_serialization_failure(500, 7, &err);
        let body = String::from_utf8(bytes)?;
        assert!(body.contains("response serialization failure"));
        assert!(body.contains("EOF"));
        Ok(())
    }
}
