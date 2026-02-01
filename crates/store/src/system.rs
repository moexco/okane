use async_trait::async_trait;
use chrono::{DateTime, Utc};
use okane_core::store::error::StoreError;
use okane_core::store::port::{Position, StockMetadata, SystemStore, User};
use sqlx::{SqlitePool, sqlite::{SqliteConnectOptions, SqlitePoolOptions}};
use std::fs;

/// 默认系统数据库存储路径
const DEFAULT_SYSTEM_DB: &str = "app.db";

/// SystemStore 的 SQLite 实现。
///
/// # Summary
/// 在中心化的 SQLite 数据库 (`app.db`) 中管理全局系统数据，包括用户、持仓和股票元数据。
///
/// # Invariants
/// * 数据库结构在存储实例创建时初始化。
/// * 所有操作均通过共享的 `SqlitePool` 执行。
pub struct SqliteSystemStore {
    pool: SqlitePool,
}

impl SqliteSystemStore {
    /// 创建新的 SqliteSystemStore 并初始化全局表结构。
    ///
    /// # Logic
    /// 1. 获取配置的数据根目录并确保其存在。
    /// 2. 配置 SQLite 连接选项，开启 `create_if_missing`。
    /// 3. 连接到数据库并执行 DDL 初始化系统表结构。
    ///
    /// # Arguments
    /// * None
    ///
    /// # Returns
    /// * `Result<Self, StoreError>` - 存储实例 or 数据库错误。
    pub async fn new() -> Result<Self, StoreError> {
        let root = crate::config::get_root_dir();
        fs::create_dir_all(&root).map_err(|e| StoreError::Database(e.to_string()))?;

        let db_path = root.join(DEFAULT_SYSTEM_DB);
        
        // 使用官方推荐的配置方式，确保自动创建数据库文件
        let options = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .connect_with(options)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

        // 初始化系统表
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS users (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                created_at DATETIME NOT NULL
            );

            CREATE TABLE IF NOT EXISTS watchlists (
                user_id TEXT NOT NULL,
                symbol TEXT NOT NULL,
                PRIMARY KEY (user_id, symbol)
            );

            CREATE TABLE IF NOT EXISTS positions (
                user_id TEXT NOT NULL,
                symbol TEXT NOT NULL,
                quantity REAL NOT NULL,
                avg_price REAL NOT NULL,
                last_updated DATETIME NOT NULL,
                PRIMARY KEY (user_id, symbol)
            );

            CREATE TABLE IF NOT EXISTS stock_metadata (
                symbol TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                exchange TEXT NOT NULL,
                sector TEXT,
                currency TEXT NOT NULL
            );
            "#,
        )
        .execute(&pool)
        .await
        .map_err(|e| StoreError::Database(e.to_string()))?;

        Ok(Self { pool })
    }
}

#[async_trait]
impl SystemStore for SqliteSystemStore {
    /// # Summary
    /// 根据 ID 获取用户信息。
    ///
    /// # Logic
    /// 查询 `users` 表。
    ///
    /// # Arguments
    /// * `id` - 用户标识符。
    ///
    /// # Returns
    /// * `Result<Option<User>, StoreError>` - 匹配的用户或 None。
    async fn get_user(&self, id: &str) -> Result<Option<User>, StoreError> {
        sqlx::query_as::<_, (String, String, DateTime<Utc>)>(
            "SELECT id, name, created_at FROM users WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::Database(e.to_string()))?
        .map(|r| {
            Ok(User {
                id: r.0,
                name: r.1,
                created_at: r.2,
            })
        })
        .transpose()
    }

    /// # Summary
    /// 保存或更新用户信息。
    ///
    /// # Logic
    /// 在 `users` 表上执行 `INSERT OR REPLACE`。
    ///
    /// # Arguments
    /// * `user` - 待保存的用户实体。
    ///
    /// # Returns
    /// * `Result<(), StoreError>`
    async fn save_user(&self, user: &User) -> Result<(), StoreError> {
        sqlx::query("INSERT OR REPLACE INTO users (id, name, created_at) VALUES (?, ?, ?)")
            .bind(&user.id)
            .bind(&user.name)
            .bind(user.created_at)
            .execute(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;
        Ok(())
    }

    /// # Summary
    /// 获取用户的自选股代码列表。
    ///
    /// # Logic
    /// 查询 `watchlists` 表中指定用户的所有记录。
    ///
    /// # Arguments
    /// * `user_id` - 用户唯一标识符。
    ///
    /// # Returns
    /// * `Result<Vec<String>, StoreError>`
    async fn get_watchlist(&self, user_id: &str) -> Result<Vec<String>, StoreError> {
        sqlx::query_scalar::<_, String>("SELECT symbol FROM watchlists WHERE user_id = ?")
            .bind(user_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))
    }

    /// # Summary
    /// 将股票添加到用户的自选列表。
    ///
    /// # Logic
    /// 向 `watchlists` 表插入记录，忽略已存在的项。
    ///
    /// # Arguments
    /// * `user_id` - 用户唯一标识符。
    /// * `symbol` - 股票代码。
    ///
    /// # Returns
    /// * `Result<(), StoreError>`
    async fn add_to_watchlist(&self, user_id: &str, symbol: &str) -> Result<(), StoreError> {
        sqlx::query("INSERT OR IGNORE INTO watchlists (user_id, symbol) VALUES (?, ?)")
            .bind(user_id)
            .bind(symbol)
            .execute(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;
        Ok(())
    }

    /// # Summary
    /// 获取用户的所有持仓。
    ///
    /// # Logic
    /// 查询 `positions` 表中指定用户的所有记录。
    ///
    /// # Arguments
    /// * `user_id` - 用户唯一标识符。
    ///
    /// # Returns
    /// * `Result<Vec<Position>, StoreError>`
    async fn get_positions(&self, user_id: &str) -> Result<Vec<Position>, StoreError> {
        let records = sqlx::query_as::<_, (String, f64, f64, DateTime<Utc>)>(
            "SELECT symbol, quantity, avg_price, last_updated FROM positions WHERE user_id = ?",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StoreError::Database(e.to_string()))?;

        Ok(records
            .into_iter()
            .map(|r| Position {
                symbol: r.0,
                quantity: r.1,
                avg_price: r.2,
                last_updated: r.3,
            })
            .collect())
    }

    /// # Summary
    /// 更新或插入持仓信息。
    ///
    /// # Logic
    /// 执行 `INSERT OR REPLACE` 写入 `positions` 表。
    ///
    /// # Arguments
    /// * `user_id` - 用户唯一标识符符。
    /// * `position` - 持仓实体。
    ///
    /// # Returns
    /// * `Result<(), StoreError>`
    async fn update_position(&self, user_id: &str, position: &Position) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT OR REPLACE INTO positions (user_id, symbol, quantity, avg_price, last_updated) VALUES (?, ?, ?, ?, ?)"
        )
        .bind(user_id)
        .bind(&position.symbol)
        .bind(position.quantity)
        .bind(position.avg_price)
        .bind(position.last_updated)
        .execute(&self.pool)
        .await
        .map_err(|e| StoreError::Database(e.to_string()))?;
        Ok(())
    }

    /// # Summary
    /// 根据代码或名称搜索股票。
    ///
    /// # Logic
    /// 对 `stock_metadata` 表进行 LIKE 模糊匹配。
    ///
    /// # Arguments
    /// * `query`: 搜索关键词（匹配 symbol 或 name）。
    ///
    /// # Returns
    /// * `Result<Vec<StockMetadata>, StoreError>` - 匹配的元数据列表。
    async fn search_stocks(&self, query: &str) -> Result<Vec<StockMetadata>, StoreError> {
        let like_query = format!("%{}%", query);
        let records = sqlx::query_as::<_, (String, String, String, Option<String>, String)>(
            "SELECT symbol, name, exchange, sector, currency FROM stock_metadata WHERE symbol LIKE ? OR name LIKE ?"
        )
        .bind(&like_query)
        .bind(&like_query)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StoreError::Database(e.to_string()))?
        .into_iter()
        .map(|r| StockMetadata {
            symbol: r.0,
            name: r.1,
            exchange: r.2,
            sector: r.3,
            currency: r.4,
        })
        .collect();

        Ok(records)
    }

    /// # Summary
    /// 保存或更新股票元数据。
    ///
    /// # Logic
    /// 执行 `INSERT OR REPLACE` 写入 `stock_metadata` 表。
    ///
    /// # Arguments
    /// * `metadata`: 股票元数据实体。
    ///
    /// # Returns
    /// * `Result<(), StoreError>`
    async fn save_stock_metadata(&self, metadata: &StockMetadata) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT OR REPLACE INTO stock_metadata (symbol, name, exchange, sector, currency) VALUES (?, ?, ?, ?, ?)"
        )
        .bind(&metadata.symbol)
        .bind(&metadata.name)
        .bind(&metadata.exchange)
        .bind(&metadata.sector)
        .bind(&metadata.currency)
        .execute(&self.pool)
        .await
        .map_err(|e| StoreError::Database(e.to_string()))?;
        Ok(())
    }
}
