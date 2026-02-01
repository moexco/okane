use super::error::StoreError;
use crate::common::{Stock, TimeFrame};
use crate::market::entity::Candle;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// # Summary
/// 用户实体，代表系统的使用者。
///
/// # Invariants
/// - `id` 必须全局唯一。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    // 用户唯一标识
    pub id: String,
    // 用户显示名称
    pub name: String,
    // 注册时间
    pub created_at: DateTime<Utc>,
}

/// # Summary
/// 持仓实体，记录用户在特定标的上的持有情况。
///
/// # Invariants
/// - `quantity` 可以为负（代表空头），但在现货模式下通常非负。
/// - `avg_price` 必须非负。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    // 股票代码
    pub symbol: String,
    // 持仓数量
    pub quantity: f64,
    // 平均持仓成本
    pub avg_price: f64,
    // 最后更新时间
    pub last_updated: DateTime<Utc>,
}

/// # Summary
/// 股票元数据实体，包含详细的静态信息。
///
/// # Invariants
/// - `symbol` 和 `exchange` 组合应唯一。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StockMetadata {
    // 股票代码
    pub symbol: String,
    // 股票全名或公司名称
    pub name: String,
    // 所属交易所
    pub exchange: String,
    // 所属板块/行业 (可选)
    pub sector: Option<String>,
    // 交易货币 (例如: USD, CNY)
    pub currency: String,
}

/// # Summary
/// 市场数据存储接口，负责 K 线数据的持久化与读取。
///
/// # Invariants
/// - 实现者应确保数据存取的原子性和一致性。
#[async_trait]
pub trait MarketStore: Send + Sync {
    /// # Summary
    /// 批量保存 K 线数据。
    ///
    /// # Logic
    /// 1. 打开证券对应的数据库连接。
    /// 2. 批量插入 K 线数据，避免重复项。
    ///
    /// # Arguments
    /// * `stock`: 目标证券实体。
    /// * `timeframe`: K 线周期。
    /// * `candles`: 待保存的数据列表。
    ///
    /// # Returns
    /// 成功返回 Ok，失败返回 `StoreError`。
    async fn save_candles(
        &self,
        stock: &Stock,
        timeframe: TimeFrame,
        candles: &[Candle],
    ) -> Result<(), StoreError>;

    /// # Summary
    /// 从存储中加载特定时间段的 K 线数据。
    ///
    /// # Logic
    /// 1. 根据 stock 和 timeframe 定位存储文件。
    /// 2. 按时间区间执行 SQL 查询。
    ///
    /// # Arguments
    /// * `stock`: 目标证券实体。
    /// * `timeframe`: K 线周期。
    /// * `start`: 开始时间。
    /// * `end`: 结束时间。
    ///
    /// # Returns
    /// 返回 K 线列表或 `StoreError`。
    async fn load_candles(
        &self,
        stock: &Stock,
        timeframe: TimeFrame,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Candle>, StoreError>;
}

/// # Summary
/// 系统级数据存储接口，负责用户、持仓及全局元数据的持久化。
///
/// # Invariants
/// - 必须保证跨表的引用完整性。
#[async_trait]
pub trait SystemStore: Send + Sync {
    // --- 用户域 ---

    /// # Summary
    /// 获取用户信息。
    ///
    /// # Logic
    /// 根据用户 ID 查询 `users` 表。
    ///
    /// # Arguments
    /// * `id`: 用户唯一标识。
    ///
    /// # Returns
    /// 存在返回 `Some(User)`，否则返回 `None`。
    async fn get_user(&self, id: &str) -> Result<Option<User>, StoreError>;

    /// # Summary
    /// 保存或更新用户信息。
    ///
    /// # Logic
    /// 执行 Upsert 操作写入 `users` 表。
    ///
    /// # Arguments
    /// * `user`: 待保存的用户实体。
    ///
    /// # Returns
    /// 操作结果。
    async fn save_user(&self, user: &User) -> Result<(), StoreError>;

    /// # Summary
    /// 获取用户的自选股列表。
    ///
    /// # Logic
    /// 查询 `watchlists` 表中指定用户的所有记录。
    ///
    /// # Arguments
    /// * `user_id`: 用户唯一标识。
    ///
    /// # Returns
    /// 返回股票代码列表。
    async fn get_watchlist(&self, user_id: &str) -> Result<Vec<String>, StoreError>;

    /// # Summary
    /// 将股票添加到用户自选股。
    ///
    /// # Logic
    /// 向 `watchlists` 表插入记录，忽略重复项。
    ///
    /// # Arguments
    /// * `user_id`: 用户唯一标识。
    /// * `symbol`: 股票代码。
    ///
    /// # Returns
    /// 操作结果。
    async fn add_to_watchlist(&self, user_id: &str, symbol: &str) -> Result<(), StoreError>;

    // --- 交易域 ---

    /// # Summary
    /// 获取用户的持仓列表。
    ///
    /// # Logic
    /// 查询 `positions` 表中指定用户的所有记录。
    ///
    /// # Arguments
    /// * `user_id`: 用户唯一标识。
    ///
    /// # Returns
    /// 返回持仓实体列表。
    async fn get_positions(&self, user_id: &str) -> Result<Vec<Position>, StoreError>;

    /// # Summary
    /// 更新持仓信息。
    ///
    /// # Logic
    /// 执行 Upsert 操作写入 `positions` 表。
    ///
    /// # Arguments
    /// * `user_id`: 用户唯一标识。
    /// * `position`: 持仓实体。
    ///
    /// # Returns
    /// 操作结果。
    async fn update_position(&self, user_id: &str, position: &Position) -> Result<(), StoreError>;

    // --- 元数据域 ---

    /// # Summary
    /// 搜索股票元数据。
    ///
    /// # Logic
    /// 在 `stock_metadata` 表中进行模糊匹配查询。
    ///
    /// # Arguments
    /// * `query`: 搜索关键词（匹配 symbol 或 name）。
    ///
    /// # Returns
    /// 匹配的元数据列表。
    async fn search_stocks(&self, query: &str) -> Result<Vec<StockMetadata>, StoreError>;

    /// # Summary
    /// 保存股票元数据。
    ///
    /// # Logic
    /// 写入 `stock_metadata` 表，用于建立索引。
    ///
    /// # Arguments
    /// * `metadata`: 股票元数据实体。
    ///
    /// # Returns
    /// 操作结果。
    async fn save_stock_metadata(&self, metadata: &StockMetadata) -> Result<(), StoreError>;
}
