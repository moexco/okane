use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use okane_core::common::{Stock, TimeFrame};
use okane_core::market::entity::Candle;
use okane_core::store::error::StoreError;
use okane_core::store::port::MarketStore;
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
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
        let base_path = crate::config::get_root_dir().join("market");
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
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS candles (
                timeframe TEXT NOT NULL,
                time DATETIME NOT NULL,
                open REAL NOT NULL,
                high REAL NOT NULL,
                low REAL NOT NULL,
                close REAL NOT NULL,
                adj_close REAL,
                volume REAL NOT NULL,
                is_final INTEGER NOT NULL,
                PRIMARY KEY (timeframe, time)
            );
            "#,
        )
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
            sqlx::query(
                r#"
                INSERT OR REPLACE INTO candles (timeframe, time, open, high, low, close, adj_close, volume, is_final)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&timeframe_str)
            .bind(candle.time)
            .bind(candle.open)
            .bind(candle.high)
            .bind(candle.low)
            .bind(candle.close)
            .bind(candle.adj_close)
            .bind(candle.volume)
            .bind(candle.is_final as i32)
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

        let records =
            sqlx::query_as::<_, (DateTime<Utc>, f64, f64, f64, f64, Option<f64>, f64, bool)>(
                r#"
            SELECT time, open, high, low, close, adj_close, volume, is_final
            FROM candles
            WHERE timeframe = ? AND time >= ? AND time <= ?
            ORDER BY time ASC
            "#,
            )
            .bind(&timeframe_str)
            .bind(start)
            .bind(end)
            .fetch_all(&pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

        Ok(records
            .into_iter()
            .map(|r| Candle {
                time: r.0,
                open: r.1,
                high: r.2,
                low: r.3,
                close: r.4,
                adj_close: r.5,
                volume: r.6,
                is_final: r.7,
            })
            .collect())
    }
}
