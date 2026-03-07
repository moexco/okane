use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;
use std::path::PathBuf;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    SqlitePool
};

use okane_core::trade::entity::{AccountId, Order, OrderDirection, OrderId, OrderStatus};
use okane_core::trade::port::{PendingOrderPort, TradeError};
use rust_decimal::Decimal;
use std::str::FromStr;

/// # Summary
/// 针对单个系统实体账户的高频高并发 SQLite 分片实现。
/// 通过一户一库 (account_<id>.db) 避免 SQLite 本身的全表写锁瓶颈。
pub struct SqlitePendingOrderStore {
    base_path: PathBuf,
    pools: DashMap<String, SqlitePool>,
}

const SQL_INIT_TABLES: &str = r#"
CREATE TABLE IF NOT EXISTS pending_orders (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    symbol TEXT NOT NULL,
    direction TEXT NOT NULL,
    price TEXT,
    volume TEXT NOT NULL,
    filled_volume TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at DATETIME NOT NULL,
    updated_at DATETIME NOT NULL
);
"#;

impl SqlitePendingOrderStore {
    pub fn new() -> Result<Self, TradeError> {
        Self::new_with_path(None)
    }

    pub fn new_with_path(root_path: Option<PathBuf>) -> Result<Self, TradeError> {
        let base_path = match root_path {
            Some(p) => p,
            None => crate::config::get_root_dir()
                .map_err(|e| TradeError::InternalError(e.to_string()))?,
        };
        
        Ok(Self {
            base_path,
            pools: DashMap::new(),
        })
    }

    /// 获取或初始化特定账户的 SQLite 连接池（直接复用 account db 名称以实现事务一致性可能更好，但这里至少保证同库同位置）
    pub async fn get_or_init_pool(&self, account_id: &str) -> Result<SqlitePool, TradeError> {
        if let Some(pool) = self.pools.get(account_id) {
            return Ok(pool.clone());
        }

        let db_name = format!("account_{}.db", account_id);
        let db_path = self.base_path.join(&db_name);

        let options = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
            .busy_timeout(std::time::Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .connect_with(options)
            .await
            .map_err(|e| TradeError::InternalError(format!("Failed to connect to SQLite: {}", e)))?;

        sqlx::query(SQL_INIT_TABLES)
            .execute(&pool)
            .await
            .map_err(|e| TradeError::InternalError(format!("Failed to init tables: {}", e)))?;

        self.pools.insert(account_id.to_string(), pool.clone());
        Ok(pool)
    }

    /// Helper to convert a sqlite row to an Order
    fn row_to_order(row: sqlx::sqlite::SqliteRow) -> Result<Order, TradeError> {
        use sqlx::Row;
        
        let direction_str: String = row.get("direction");
        let direction = match direction_str.as_str() {
            "Buy" => OrderDirection::Buy,
            "Sell" => OrderDirection::Sell,
            _ => return Err(TradeError::InternalError(format!("Invalid direction: {}", direction_str))),
        };

        let status_str: String = row.get("status");
        let status = match status_str.as_str() {
            "Pending" => OrderStatus::Pending,
            "Submitted" => OrderStatus::Submitted,
            "PartialFilled" => OrderStatus::PartialFilled,
            "Filled" => OrderStatus::Filled,
            "Canceled" => OrderStatus::Canceled,
            "Rejected" => OrderStatus::Rejected,
            _ => return Err(TradeError::InternalError(format!("Invalid status: {}", status_str))),
        };

        let price_str: Option<String> = row.get("price");
        let price = match price_str {
            Some(p) => Some(Decimal::from_str(&p).map_err(|_| TradeError::InternalError("Price decimal parse error".to_string()))?),
            None => None,
        };

        let volume_str: String = row.get("volume");
        let volume = Decimal::from_str(&volume_str).map_err(|_| TradeError::InternalError("Volume decimal parse error".to_string()))?;

        let filled_volume_str: String = row.get("filled_volume");
        let filled_volume = Decimal::from_str(&filled_volume_str).map_err(|_| TradeError::InternalError("Filled Volume decimal parse error".to_string()))?;

        // 如果数据库中的时间戳解析失败，必须显式抛出错误以防止策略回测逻辑被静默误导。
        let created_at: chrono::DateTime<Utc> = row.try_get("created_at")
            .map_err(|e| TradeError::InternalError(format!("Failed to parse created_at: {}", e)))?;

        Ok(Order {
            id: OrderId(row.get("id")),
            account_id: AccountId(row.get("account_id")),
            symbol: row.get("symbol"),
            direction,
            price,
            volume,
            filled_volume,
            status,
            created_at: created_at.timestamp_millis(),
        })
    }
}

#[async_trait]
impl PendingOrderPort for SqlitePendingOrderStore {
    async fn save(&self, order: Order) -> Result<(), TradeError> {
        let pool = self.get_or_init_pool(&order.account_id.0).await?;
        
        let price_str = order.price.map(|p| p.to_string());
        let dir_str = match order.direction {
            OrderDirection::Buy => "Buy",
            OrderDirection::Sell => "Sell",
        };
        let status_str = match order.status {
            OrderStatus::Pending => "Pending",
            OrderStatus::Submitted => "Submitted",
            OrderStatus::PartialFilled => "PartialFilled",
            OrderStatus::Filled => "Filled",
            OrderStatus::Canceled => "Canceled",
            OrderStatus::Rejected => "Rejected",
        };
        
        let now = Utc::now();
        
        sqlx::query(
            "INSERT INTO pending_orders (id, account_id, symbol, direction, price, volume, filled_volume, status, created_at, updated_at) 
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET 
                filled_volume=excluded.filled_volume,
                status=excluded.status,
                updated_at=excluded.updated_at
            ")
            .bind(&order.id.0)
            .bind(&order.account_id.0)
            .bind(&order.symbol)
            .bind(dir_str)
            .bind(price_str)
            .bind(order.volume.to_string())
            .bind(order.filled_volume.to_string())
            .bind(status_str)
            .bind(now)  // Since creation time is immutable in DB context, we just bind it to upsert
            .bind(now)
            .execute(&pool)
            .await
            .map_err(|e| TradeError::InternalError(e.to_string()))?;
            
        Ok(())
    }

    async fn remove(&self, order_id: &OrderId) -> Result<Option<Order>, TradeError> {
        // 由于分库，移除时如果不知道 account_id 就必须遍历所有的池并尝试删除
        // 优化方法是在核心对象里增加一个 global routing 表，但为了简单，这里直接遍历内存已缓存的池
        let mut target_order = None;
        for entry in self.pools.iter() {
            let pool = entry.value();
            
            // 尝试查询以获取将要删除的实体
            let row_opt = sqlx::query("SELECT * FROM pending_orders WHERE id = ?")
                .bind(&order_id.0)
                .fetch_optional(pool)
                .await
                .map_err(|e| TradeError::InternalError(e.to_string()))?;
                
            if let Some(row) = row_opt {
                target_order = Some(Self::row_to_order(row)?);
                // 删除
                sqlx::query("DELETE FROM pending_orders WHERE id = ?")
                    .bind(&order_id.0)
                    .execute(pool)
                    .await
                    .map_err(|e| TradeError::InternalError(e.to_string()))?;
                break;
            }
        }
        
        Ok(target_order)
    }

    async fn get(&self, order_id: &OrderId) -> Result<Option<Order>, TradeError> {
        for entry in self.pools.iter() {
            let pool = entry.value();
            let row_opt = sqlx::query("SELECT * FROM pending_orders WHERE id = ?")
                .bind(&order_id.0)
                .fetch_optional(pool)
                .await
                .map_err(|e| TradeError::InternalError(e.to_string()))?;
                
            if let Some(row) = row_opt {
                return Ok(Some(Self::row_to_order(row)?));
            }
        }
        Ok(None)
    }

    async fn get_by_account(&self, account_id: &AccountId) -> Result<Vec<Order>, TradeError> {
        let pool = self.get_or_init_pool(&account_id.0).await?;
        let rows = sqlx::query("SELECT * FROM pending_orders WHERE account_id = ?")
            .bind(&account_id.0)
            .fetch_all(&pool)
            .await
            .map_err(|e| TradeError::InternalError(e.to_string()))?;
            
        let mut orders = Vec::new();
        for row in rows {
            orders.push(Self::row_to_order(row)?);
        }
        Ok(orders)
    }

    async fn get_by_symbol(&self, symbol: &str) -> Result<Vec<Order>, TradeError> {
        let mut orders = Vec::new();
        for entry in self.pools.iter() {
            let pool = entry.value();
            let rows = sqlx::query("SELECT * FROM pending_orders WHERE symbol = ?")
                .bind(symbol)
                .fetch_all(pool)
                .await
                .map_err(|e| TradeError::InternalError(e.to_string()))?;
                
            for row in rows {
                orders.push(Self::row_to_order(row)?);
            }
        }
        Ok(orders)
    }

    async fn update_status(&self, order_id: &OrderId, status: OrderStatus) -> Result<(), TradeError> {
        let status_str = match status {
            OrderStatus::Pending => "Pending",
            OrderStatus::Submitted => "Submitted",
            OrderStatus::PartialFilled => "PartialFilled",
            OrderStatus::Filled => "Filled",
            OrderStatus::Canceled => "Canceled",
            OrderStatus::Rejected => "Rejected",
        };
        
        for entry in self.pools.iter() {
            let pool = entry.value();
            let res = sqlx::query("UPDATE pending_orders SET status = ?, updated_at = ? WHERE id = ?")
                .bind(status_str)
                .bind(Utc::now())
                .bind(&order_id.0)
                .execute(pool)
                .await
                .map_err(|e| TradeError::InternalError(e.to_string()))?;
                
            if res.rows_affected() > 0 {
                return Ok(());
            }
        }
        Ok(())
    }
}
