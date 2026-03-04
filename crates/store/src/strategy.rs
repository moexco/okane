use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use okane_core::store::error::StoreError;
use okane_core::strategy::entity::{StrategyInstance, StrategyStatus};
use okane_core::strategy::port::StrategyStore;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    SqlitePool,
};
use std::path::PathBuf;

/// # Summary
/// StrategyStore 的 SQLite 实现，采用"一户一库"策略。
///
/// # Invariants
/// * 每个用户拥有独立的 SQLite 数据库文件 (`strategy/<user_id>.db`)。
/// * 连接池按 user_id 缓存。
pub struct SqliteStrategyStore {
    base_path: PathBuf,
    pools: DashMap<String, SqlitePool>,
}

const SQL_INIT_TABLES: &str = r#"
CREATE TABLE IF NOT EXISTS strategy_instances (
    id TEXT PRIMARY KEY,
    symbol TEXT NOT NULL,
    account_id TEXT NOT NULL DEFAULT '',
    timeframe TEXT NOT NULL,
    engine_type TEXT NOT NULL,
    source BLOB NOT NULL,
    status TEXT NOT NULL,
    created_at DATETIME NOT NULL,
    updated_at DATETIME NOT NULL
);
"#;

const SQL_INSERT_STRATEGY: &str = r#"
INSERT OR REPLACE INTO strategy_instances 
(id, symbol, account_id, timeframe, engine_type, source, status, created_at, updated_at)
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
"#;

const SQL_UPDATE_STATUS: &str = "UPDATE strategy_instances SET status = ?, updated_at = ? WHERE id = ?";

const SQL_SELECT_STRATEGY: &str = "SELECT * FROM strategy_instances WHERE id = ?";

const SQL_SELECT_ALL_STRATEGIES: &str = "SELECT * FROM strategy_instances";

const SQL_DELETE_STRATEGY: &str = "DELETE FROM strategy_instances WHERE id = ?";


impl SqliteStrategyStore {
    /// # Summary
    /// 创建新的 SqliteStrategyStore 实例。
    pub fn new() -> Result<Self, StoreError> {
        let base_path = crate::config::get_root_dir()?.join("strategy");
        if !base_path.exists() {
            std::fs::create_dir_all(&base_path).map_err(|e| StoreError::Database(e.to_string()))?;
        }
        Ok(Self {
            base_path,
            pools: DashMap::new(),
        })
    }

    async fn get_or_init_pool(&self, user_id: &str) -> Result<SqlitePool, StoreError> {
        if let Some(pool) = self.pools.get(user_id) {
            return Ok(pool.clone());
        }

        let db_path = self.base_path.join(format!("strategy_{}.db", user_id));
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

        sqlx::query(SQL_INIT_TABLES)
        .execute(&pool)
        .await
        .map_err(|e| StoreError::Database(e.to_string()))?;

        self.pools.insert(user_id.to_string(), pool.clone());
        Ok(pool)
    }
}

fn status_to_str(s: &StrategyStatus) -> String {
    match s {
        StrategyStatus::Pending => "Pending".to_string(),
        StrategyStatus::Running => "Running".to_string(),
        StrategyStatus::Stopped => "Stopped".to_string(),
        StrategyStatus::Failed(msg) => format!("Failed:{}", msg),
    }
}

fn str_to_status(s: &str) -> StrategyStatus {
    match s {
        "Pending" => StrategyStatus::Pending,
        "Running" => StrategyStatus::Running,
        "Stopped" => StrategyStatus::Stopped,
        other if other.starts_with("Failed:") => {
            StrategyStatus::Failed(other.strip_prefix("Failed:").unwrap_or("").to_string())
        }
        _ => StrategyStatus::Failed(format!("Unknown status: {}", s)),
    }
}

#[async_trait]
impl StrategyStore for SqliteStrategyStore {
    async fn save_instance(
        &self,
        user_id: &str,
        instance: &StrategyInstance,
    ) -> Result<(), StoreError> {
        let pool = self.get_or_init_pool(user_id).await?;
        sqlx::query(SQL_INSERT_STRATEGY)
        .bind(&instance.id)
        .bind(&instance.symbol)
        .bind(&instance.account_id)
        .bind(instance.timeframe.to_string())
        .bind(instance.engine_type.to_string())
        .bind(&instance.source)
        .bind(status_to_str(&instance.status))
        .bind(instance.created_at)
        .bind(instance.updated_at)
        .execute(&pool)
        .await
        .map_err(|e| StoreError::Database(e.to_string()))?;
        Ok(())
    }

    async fn get_instance(&self, user_id: &str, id: &str) -> Result<StrategyInstance, StoreError> {
        let pool = self.get_or_init_pool(user_id).await?;
        let row = sqlx::query_as::<_, (String, String, String, String, String, Vec<u8>, String, DateTime<Utc>, DateTime<Utc>)>(
            SQL_SELECT_STRATEGY
        )
        .bind(id)
        .fetch_optional(&pool)
        .await
        .map_err(|e| StoreError::Database(e.to_string()))?
        .ok_or(StoreError::NotFound)?;

        Ok(StrategyInstance {
            id: row.0,
            symbol: row.1,
            account_id: row.2,
            timeframe: row.3.parse().map_err(|e: String| StoreError::Database(e))?,
            engine_type: row.4.parse().map_err(|e: String| StoreError::Database(e))?,
            source: row.5,
            status: str_to_status(&row.6),
            created_at: row.7,
            updated_at: row.8,
        })
    }

    async fn update_status(&self, user_id: &str, id: &str, status: StrategyStatus) -> Result<(), StoreError> {
        let pool = self.get_or_init_pool(user_id).await?;
        sqlx::query(SQL_UPDATE_STATUS)
            .bind(status_to_str(&status))
            .bind(Utc::now())
            .bind(id)
            .execute(&pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;
        Ok(())
    }

    async fn list_instances(&self, user_id: &str) -> Result<Vec<StrategyInstance>, StoreError> {
        let pool = self.get_or_init_pool(user_id).await?;
        let rows = sqlx::query_as::<_, (String, String, String, String, String, Vec<u8>, String, DateTime<Utc>, DateTime<Utc>)> (
            SQL_SELECT_ALL_STRATEGIES
        )
        .fetch_all(&pool)
        .await
        .map_err(|e| StoreError::Database(e.to_string()))?;

        rows.into_iter().map(|row| {
             Ok(StrategyInstance {
                id: row.0,
                symbol: row.1,
                account_id: row.2,
                timeframe: row.3.parse().map_err(|e: String| StoreError::Database(e))?,
                engine_type: row.4.parse().map_err(|e: String| StoreError::Database(e))?,
                source: row.5,
                status: str_to_status(&row.6),
                created_at: row.7,
                updated_at: row.8,
            })
        }).collect()
    }

    async fn delete_instance(&self, user_id: &str, id: &str) -> Result<(), StoreError> {
        let pool = self.get_or_init_pool(user_id).await?;
        let res = sqlx::query(SQL_DELETE_STRATEGY)
            .bind(id)
            .execute(&pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;
        
        if res.rows_affected() == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }
}
