use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use okane_core::common::{Stock, TimeFrame};
use okane_core::market::entity::Candle;
use okane_core::store::error::StoreError;
use okane_core::store::port::MarketStore;
use rust_decimal::Decimal;
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteRow},
};
use std::path::PathBuf;

/// MarketStore 的 SQLite 实现，采用“一库一股”策略。
///
/// # Summary
/// 为每个股票维护一个独立的 SQLite 数据库文件，以实现物理数据隔离。
///
/// # Invariants
/// * 数据库文件存储在指定的 `base_path` 目录下。
/// * 连接池被缓存以避免频繁的文件打开操作。
pub struct SqliteMarketStore {
    base_path: PathBuf,
    pools: DashMap<String, SqlitePool>,
}

const SQL_INIT_TABLES: &str = r#"
CREATE TABLE IF NOT EXISTS candles (
    timeframe TEXT NOT NULL,
    time DATETIME NOT NULL,
    open TEXT NOT NULL,
    high TEXT NOT NULL,
    low TEXT NOT NULL,
    close TEXT NOT NULL,
    adj_close TEXT,
    volume TEXT NOT NULL,
    is_final INTEGER NOT NULL,
    PRIMARY KEY (timeframe, time)
);
"#;

const SQL_INSERT_CANDLE: &str = r#"
INSERT OR REPLACE INTO candles (timeframe, time, open, high, low, close, adj_close, volume, is_final)
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
"#;

const SQL_SELECT_CANDLES: &str =
    "SELECT * FROM candles WHERE timeframe = ? AND time >= ? AND time <= ? ORDER BY time ASC";

impl SqliteMarketStore {
    /// 创建新的 SqliteMarketStore 实例。
    ///
    /// # Logic
    /// 1. 获取配置的数据根目录下的 `market` 子目录。
    /// 2. 确保该目录存在。
    ///
    /// # Arguments
    /// * None
    ///
    /// # Returns
    /// * `Result<Self, StoreError>` - 存储实例或错误。
    pub fn new() -> Result<Self, StoreError> {
        Self::new_with_path(None)
    }

    /// 创建新的 SqliteMarketStore 实例，支持指定路径。
    pub fn new_with_path(root_path: Option<PathBuf>) -> Result<Self, StoreError> {
        let base_path = match root_path {
            Some(p) => p.join("market"),
            None => crate::config::get_root_dir()?.join("market"),
        };
        if !base_path.exists() {
            std::fs::create_dir_all(&base_path).map_err(|e| StoreError::Database(e.to_string()))?;
        }
        Ok(Self {
            base_path,
            pools: DashMap::new(),
        })
    }

    /// 获取或初始化特定股票的连接池。
    ///
    /// # Logic
    /// 1. 根据股票代码和交易所生成文件名。
    /// 2. 配置 SQLite 连接选项，开启 `create_if_missing`。
    /// 3. 如果缓存中没有，则创建新连接池并运行初始化建表 SQL。
    async fn get_or_init_pool(&self, stock: &Stock) -> Result<SqlitePool, StoreError> {
        // 对于外部行情源，如果交易所信息缺失，回退到 "UNKNOWN" 以保证数据库文件能被定位。
        // OK: External Feed metadata fallback
        let exchange = stock.exchange.as_deref().unwrap_or("UNKNOWN");
        let key = format!("{}_{}", stock.symbol, exchange);

        if let Some(pool) = self.pools.get(&key) {
            return Ok(pool.clone());
        }

        let db_path = self.base_path.join(format!("{}.db", key));

        // 使用官方推荐的配置方式，确保自动创建数据库文件
        let options = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .connect_with(options)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

        // 初始化个股 K 下表 (同步 Candle 最新结构)
        sqlx::query(SQL_INIT_TABLES)
            .execute(&pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

        self.pools.insert(key, pool.clone());
        Ok(pool)
    }
}

#[async_trait]
impl MarketStore for SqliteMarketStore {
    /// # Summary
    /// 批量保存 K 线数据。
    ///
    /// # Logic
    /// 1. 获取个股连接池。
    /// 2. 执行批量 `INSERT OR REPLACE`。
    ///
    /// # Arguments
    /// * `stock` - 目标证券。
    /// * `timeframe` - 周期。
    /// * `candles` - 数据列表。
    ///
    /// # Returns
    /// * `Result<(), StoreError>`
    async fn save_candles(
        &self,
        stock: &Stock,
        timeframe: TimeFrame,
        candles: &[Candle],
    ) -> Result<(), StoreError> {
        let pool = self.get_or_init_pool(stock).await?;
        let timeframe_str = format!("{:?}", timeframe);

        for candle in candles {
            sqlx::query(SQL_INSERT_CANDLE)
                .bind(&timeframe_str)
                .bind(candle.time)
                .bind(candle.open.to_string())
                .bind(candle.high.to_string())
                .bind(candle.low.to_string())
                .bind(candle.close.to_string())
                .bind(candle.adj_close.map(|a| a.to_string()))
                .bind(candle.volume.to_string())
                .bind(i32::from(candle.is_final))
                .execute(&pool)
                .await
                .map_err(|e| StoreError::Database(e.to_string()))?;
        }

        Ok(())
    }

    /// # Summary
    /// 加载 K 线数据。
    ///
    /// # Logic
    /// 1. 获取个股连接池。
    /// 2. 按时间区间查询 `candles` 表。
    ///
    /// # Arguments
    /// * `stock` - 目标证券。
    /// * `timeframe` - 周期。
    /// * `start` - 开始时间。
    /// * `end` - 结束时间。
    ///
    /// # Returns
    /// * `Result<Vec<Candle>, StoreError>`
    async fn load_candles(
        &self,
        stock: &Stock,
        timeframe: TimeFrame,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Candle>, StoreError> {
        let pool = self.get_or_init_pool(stock).await?;
        let timeframe_str = format!("{:?}", timeframe);

        let rows: Vec<SqliteRow> = sqlx::query(SQL_SELECT_CANDLES)
            .bind(&timeframe_str)
            .bind(start)
            .bind(end)
            .fetch_all(&pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            use sqlx::Row;
            use std::str::FromStr;
            let open_str: String = row.get("open");
            let high_str: String = row.get("high");
            let low_str: String = row.get("low");
            let close_str: String = row.get("close");
            let adj_close_str: Option<String> = row.get("adj_close");
            let volume_str: String = row.get("volume");

            results.push(Candle {
                time: row.get("time"),
                open: Decimal::from_str(&open_str).map_err(|e| {
                    StoreError::Database(format!("Failed to parse Decimal '{}': {}", open_str, e))
                })?,
                high: Decimal::from_str(&high_str).map_err(|e| {
                    StoreError::Database(format!("Failed to parse Decimal '{}': {}", high_str, e))
                })?,
                low: Decimal::from_str(&low_str).map_err(|e| {
                    StoreError::Database(format!("Failed to parse Decimal '{}': {}", low_str, e))
                })?,
                close: Decimal::from_str(&close_str).map_err(|e| {
                    StoreError::Database(format!("Failed to parse Decimal '{}': {}", close_str, e))
                })?,
                adj_close: adj_close_str.and_then(|s| Decimal::from_str(&s).ok()),
                volume: Decimal::from_str(&volume_str).map_err(|e| {
                    StoreError::Database(format!("Failed to parse Decimal '{}': {}", volume_str, e))
                })?,
                is_final: row.get::<i32, _>("is_final") != 0,
            });
        }
        Ok(results)
    }
}
