use async_trait::async_trait;
use chrono::{DateTime, Utc};
use okane_core::store::error::StoreError;
use okane_core::store::port::{Position, StockMetadata, SystemStore, User};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use rust_decimal::Decimal;
use std::fs;

/// 默认系统数据库存储路径
const DEFAULT_SYSTEM_DB: &str = "system.db";

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

const SQL_INIT_TABLES: &str = r#"
CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'Standard',
    force_password_change BOOLEAN NOT NULL DEFAULT FALSE,
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
    quantity TEXT NOT NULL,
    avg_price TEXT NOT NULL,
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

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at DATETIME NOT NULL
);
"#;

const SQL_SELECT_USER: &str = "SELECT * FROM users WHERE id = ?";
const SQL_INSERT_USER: &str = r#"
INSERT OR REPLACE INTO users (id, name, password_hash, role, force_password_change, created_at)
VALUES (?, ?, ?, ?, ?, ?)
"#;

const SQL_SELECT_WATCHLIST: &str = "SELECT symbol FROM watchlists WHERE user_id = ?";
const SQL_INSERT_WATCHLIST: &str = "INSERT OR IGNORE INTO watchlists (user_id, symbol) VALUES (?, ?)";
const SQL_DELETE_WATCHLIST: &str = "DELETE FROM watchlists WHERE user_id = ? AND symbol = ?";

const SQL_SELECT_POSITIONS: &str = "SELECT symbol, quantity, avg_price, last_updated FROM positions WHERE user_id = ?";
const SQL_UPDATE_POSITION: &str = r#"
INSERT OR REPLACE INTO positions (user_id, symbol, quantity, avg_price, last_updated)
VALUES (?, ?, ?, ?, ?)
"#;

const SQL_SEARCH_STOCKS: &str = "SELECT * FROM stock_metadata WHERE symbol LIKE ? OR name LIKE ?";
const SQL_INSERT_METADATA: &str = r#"
INSERT OR REPLACE INTO stock_metadata (symbol, name, exchange, sector, currency)
VALUES (?, ?, ?, ?, ?)
"#;

const SQL_SELECT_SETTING: &str = "SELECT value FROM settings WHERE key = ?";
const SQL_INSERT_SETTING: &str = "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES (?, ?, ?)";
const SQL_COUNT_USERS: &str = "SELECT COUNT(*) FROM users";

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
        let root = crate::config::get_root_dir()?;
        fs::create_dir_all(&root).map_err(|e| StoreError::Database(e.to_string()))?;

        let db_path = root.join(DEFAULT_SYSTEM_DB);

        // 使用官方推荐的配置方式，确保自动创建数据库文件
        let options = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
            .busy_timeout(std::time::Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .connect_with(options)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

        // 初始化系统表
        sqlx::query(SQL_INIT_TABLES)
        .execute(&pool)
        .await
        .map_err(|e| StoreError::Database(e.to_string()))?;

        let count: (i64,) = sqlx::query_as(SQL_COUNT_USERS)
            .fetch_one(&pool)
            .await
            .unwrap_or((0,));

        if count.0 == 0 {
            // 生成 12 位随机密码
            let pwd: String = uuid::Uuid::new_v4().to_string()[..12].to_string();
            
            let hashed = bcrypt::hash(&pwd, bcrypt::DEFAULT_COST)
                .map_err(|e| StoreError::Database(format!("Failed to hash password: {}", e)))?;
            
            sqlx::query(SQL_INSERT_USER)
                .bind("admin")
                .bind("System Administrator")
                .bind(hashed)
                .bind("Admin")
                .bind(true) // 首次登录强制修改密码
                .bind(Utc::now())
                .execute(&pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;
                
            tracing::warn!("=====================================================");
            tracing::warn!("🔒 CRITICAL: INITIALIZED SYSTEM ADMIN ACCOUNT 🔒");
            tracing::warn!("Username: admin");
            tracing::warn!("Password: {}", pwd);
            tracing::warn!("⚠️ PLEASE CHANGE THIS PASSWORD UPON FIRST LOGIN ⚠️");
            tracing::warn!("=====================================================");
        }

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
        let result = sqlx::query_as::<_, (String, String, String, String, bool, DateTime<Utc>)>(
            SQL_SELECT_USER,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::Database(e.to_string()))?;

        match result {
            Some(r) => {
                let role = r.3.parse().map_err(|e| StoreError::Database(format!("Invalid role: {}", e)))?;
                Ok(Some(User {
                    id: r.0,
                    name: r.1,
                    password_hash: r.2,
                    role,
                    force_password_change: r.4,
                    created_at: r.5,
                }))
            }
            None => Ok(None),
        }
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
        sqlx::query(SQL_INSERT_USER)
            .bind(&user.id)
            .bind(&user.name)
            .bind(&user.password_hash)
            .bind(user.role.to_string())
            .bind(user.force_password_change)
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
        sqlx::query_scalar::<_, String>(SQL_SELECT_WATCHLIST)
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
        sqlx::query(SQL_INSERT_WATCHLIST)
            .bind(user_id)
            .bind(symbol)
            .execute(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;
        Ok(())
    }

    /// # Summary
    /// 将股票从用户的自选列表移除。
    ///
    /// # Logic
    /// 从 `watchlists` 表删除相关记录。
    ///
    /// # Arguments
    /// * `user_id` - 用户唯一标识符。
    /// * `symbol` - 股票代码。
    ///
    /// # Returns
    /// * `Result<(), StoreError>`
    async fn remove_from_watchlist(&self, user_id: &str, symbol: &str) -> Result<(), StoreError> {
        sqlx::query(SQL_DELETE_WATCHLIST)
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
        let records = sqlx::query_as::<_, (String, String, String, DateTime<Utc>)>(SQL_SELECT_POSITIONS)
            .bind(user_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

        Ok(records
            .into_iter()
            .map(|r| {
                use std::str::FromStr;
                Position {
                    symbol: r.0,
                    quantity: Decimal::from_str(&r.1).unwrap_or_default(),
                    avg_price: Decimal::from_str(&r.2).unwrap_or_default(),
                    last_updated: r.3,
                }
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
        sqlx::query(SQL_UPDATE_POSITION)
            .bind(user_id)
            .bind(&position.symbol)
            .bind(position.quantity.to_string())
            .bind(position.avg_price.to_string())
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
        let records = sqlx::query_as::<_, (String, String, String, Option<String>, String)>(SQL_SEARCH_STOCKS)
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
        sqlx::query(SQL_INSERT_METADATA)
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

    async fn get_setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        sqlx::query_scalar::<_, String>(SQL_SELECT_SETTING)
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))
    }

    async fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        sqlx::query(SQL_INSERT_SETTING)
            .bind(key)
            .bind(value)
            .bind(Utc::now())
            .execute(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;
        Ok(())
    }
}
