use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use okane_core::store::error::StoreError;
use okane_core::strategy::entity::{StrategyInstance, StrategyLogEntry, StrategyStatus};
use okane_core::strategy::port::{StrategyLogPort, StrategyStore};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    Row, SqlitePool,
};
use std::io::SeekFrom;
use tokio::fs::OpenOptions;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
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

CREATE TABLE IF NOT EXISTS strategy_log_index (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    strategy_id TEXT NOT NULL,
    timestamp DATETIME NOT NULL,
    level TEXT NOT NULL,
    offset INTEGER NOT NULL,
    length INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_strategy_log_id_time ON strategy_log_index(strategy_id, timestamp);
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
        Self::new_with_path(None)
    }

    /// 创建新的 SqliteStrategyStore 实例，支持指定路径。
    pub fn new_with_path(root_path: Option<PathBuf>) -> Result<Self, StoreError> {
        let base_path = match root_path {
            Some(p) => p.join("strategy"),
            None => crate::config::get_root_dir()?.join("strategy"),
        };
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

// 移除本地 status_to_str 和 str_to_status 辅助函数，直接使用 StrategyStatus 的 Display 和 FromStr 实现。

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
        .bind(instance.status.to_string())
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
            status: row.6.parse().unwrap_or_else(|e| StrategyStatus::Failed(format!("Parse error: {}", e))),
            created_at: row.7,
            updated_at: row.8,
        })
    }

    async fn update_status(&self, user_id: &str, id: &str, status: StrategyStatus) -> Result<(), StoreError> {
        let pool = self.get_or_init_pool(user_id).await?;
        sqlx::query(SQL_UPDATE_STATUS)
            .bind(status.to_string())
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
                status: row.6.parse().unwrap_or_else(|e| StrategyStatus::Failed(format!("Parse error: {}", e))),
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

#[async_trait]
impl StrategyLogPort for SqliteStrategyStore {
    async fn append_log(
        &self,
        user_id: &str,
        entry: &StrategyLogEntry,
    ) -> Result<(), StoreError> {
        let pool = self.get_or_init_pool(user_id).await?;
        
        // 1. 确定物理文件路径
        let log_dir = self.base_path.join("logs").join(user_id);
        if !log_dir.exists() {
            std::fs::create_dir_all(&log_dir).map_err(|e| StoreError::Database(e.to_string()))?;
        }
        let log_path = log_dir.join(format!("{}.logl", entry.strategy_id));

        // 2. 序列化并写入文件
        let mut json = serde_json::to_vec(entry).map_err(|e| StoreError::Database(e.to_string()))?;
        json.push(b'\n');
        let length: i64 = json.len().try_into().map_err(|_| StoreError::Database("Log entry too large".to_string()))?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;
        
        let offset_u = file.seek(SeekFrom::End(0)).await.map_err(|e| StoreError::Database(e.to_string()))?;
        let offset: i64 = offset_u.try_into().map_err(|_| StoreError::Database("File offset too large".to_string()))?;
        
        file.write_all(&json).await.map_err(|e| StoreError::Database(e.to_string()))?;
        file.flush().await.map_err(|e| StoreError::Database(e.to_string()))?;

        // 3. 写入 SQLite 索引
        sqlx::query("INSERT INTO strategy_log_index (strategy_id, timestamp, level, offset, length) VALUES (?, ?, ?, ?, ?)")
            .bind(&entry.strategy_id)
            .bind(entry.timestamp)
            .bind(entry.level.to_string())
            .bind(offset)
            .bind(length)
            .execute(&pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

        Ok(())
    }

    async fn query_logs(
        &self,
        user_id: &str,
        strategy_id: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<StrategyLogEntry>, StoreError> {
        let pool = self.get_or_init_pool(user_id).await?;

        let i_limit: i64 = limit.try_into().map_err(|_| StoreError::Database("Limit too large".to_string()))?;
        let i_offset: i64 = offset.try_into().map_err(|_| StoreError::Database("Offset too large".to_string()))?;

        // 1. 从索引表中查找偏移量
        let rows = sqlx::query("SELECT offset, length FROM strategy_log_index WHERE strategy_id = ? ORDER BY timestamp DESC LIMIT ? OFFSET ?")
            .bind(strategy_id)
            .bind(i_limit)
            .bind(i_offset)
            .fetch_all(&pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

        if rows.is_empty() {
            return Ok(vec![]);
        }

        // 2. 从物理文件中批量读取
        let log_path = self.base_path.join("logs").join(user_id).join(format!("{}.logl", strategy_id));
        let mut file = tokio::fs::File::open(&log_path).await.map_err(|e| StoreError::Database(e.to_string()))?;
        
        let mut entries = Vec::new();
        for row in rows {
            let off: i64 = row.get(0);
            let len: i64 = row.get(1);
            
            let u_off: u64 = off.try_into().map_err(|_| StoreError::Database("Negative log offset".to_string()))?;
            let u_len: usize = len.try_into().map_err(|_| StoreError::Database("Negative log length".to_string()))?;

            let mut buf = vec![0u8; u_len];
            file.seek(SeekFrom::Start(u_off)).await.map_err(|e| StoreError::Database(e.to_string()))?;
            file.read_exact(&mut buf).await.map_err(|e| StoreError::Database(e.to_string()))?;
            
            let entry: StrategyLogEntry = serde_json::from_slice(&buf).map_err(|e| StoreError::Database(e.to_string()))?;
            entries.push(entry);
        }

        Ok(entries)
    }
}
