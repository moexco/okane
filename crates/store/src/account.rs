use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;
use okane_core::trade::entity::{AccountId, AccountSnapshot, OrderDirection, Position, Trade};
use okane_core::trade::port::{AccountPort, TradeError};
use rust_decimal::Decimal;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    SqlitePool,
};
use std::path::PathBuf;
use std::str::FromStr;
use tracing::{info, warn};

/// # Summary
/// 针对单个系统实体账户的高频高并发 SQLite 分片实现。
/// 通过一户一库 (account_<id>.db) 避免 SQLite 本身的全表写锁瓶颈。
pub struct SqliteAccountStore {
    base_path: PathBuf,
    pools: DashMap<String, SqlitePool>,
}

impl SqliteAccountStore {
    pub fn new() -> Result<Self, TradeError> {
        let base_path = crate::config::get_root_dir().join("accounts");
        if !base_path.exists() {
            std::fs::create_dir_all(&base_path)
                .map_err(|e| TradeError::InternalError(format!("Failed to create account dir: {}", e)))?;
        }
        Ok(Self {
            base_path,
            pools: DashMap::new(),
        })
    }

    /// 获取特定账户的 DB 实例
    pub async fn get_or_init_pool(&self, account_id: &str) -> Result<SqlitePool, TradeError> {
        if let Some(pool) = self.pools.get(account_id) {
            return Ok(pool.clone());
        }

        let db_path = self.base_path.join(format!("account_{}.db", account_id));
        let options = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
            .busy_timeout(std::time::Duration::from_secs(10));

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(|e| TradeError::InternalError(e.to_string()))?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS asset_status (
                id TEXT PRIMARY KEY,
                available_balance TEXT NOT NULL,
                frozen_balance TEXT NOT NULL,
                updated_at DATETIME NOT NULL
            );

            CREATE TABLE IF NOT EXISTS positions (
                symbol TEXT PRIMARY KEY,
                quantity TEXT NOT NULL,
                avg_price TEXT NOT NULL,
                updated_at DATETIME NOT NULL
            );

            CREATE TABLE IF NOT EXISTS trade_ledger (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                action_type TEXT NOT NULL,
                asset_change TEXT NOT NULL,
                frozen_change TEXT NOT NULL,
                created_at DATETIME NOT NULL
            );
            "#,
        )
        .execute(&pool)
        .await
        .map_err(|e| TradeError::InternalError(e.to_string()))?;

        // 初始化默认的 MAIN 资产槽位
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO asset_status (id, available_balance, frozen_balance, updated_at)
            VALUES ('MAIN', '0', '0', ?)
            "#,
        )
        .bind(Utc::now())
        .execute(&pool)
        .await
        .map_err(|e| TradeError::InternalError(e.to_string()))?;

        self.pools.insert(account_id.to_string(), pool.clone());
        Ok(pool)
    }

    /// 后台充值接口 (供系统管理或测试接入资本)
    pub async fn deposit(&self, account_id: &AccountId, amount: Decimal) -> Result<(), TradeError> {
        let pool = self.get_or_init_pool(&account_id.0).await?;
        let mut tx = pool.begin().await.map_err(|e| TradeError::InternalError(e.to_string()))?;

        let row: (String, String) = sqlx::query_as("SELECT available_balance, frozen_balance FROM asset_status WHERE id = 'MAIN'")
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| TradeError::InternalError(e.to_string()))?;

        let mut avail = Decimal::from_str(&row.0).unwrap_or_default();
        avail += amount;

        sqlx::query("UPDATE asset_status SET available_balance = ?, updated_at = ? WHERE id = 'MAIN'")
            .bind(avail.to_string())
            .bind(Utc::now())
            .execute(&mut *tx)
            .await
            .map_err(|e| TradeError::InternalError(e.to_string()))?;

        sqlx::query("INSERT INTO trade_ledger (action_type, asset_change, frozen_change, created_at) VALUES (?, ?, ?, ?)")
            .bind("Deposit")
            .bind(amount.to_string())
            .bind("0")
            .bind(Utc::now())
            .execute(&mut *tx)
            .await
            .map_err(|e| TradeError::InternalError(e.to_string()))?;

        tx.commit().await.map_err(|e| TradeError::InternalError(e.to_string()))?;
        info!("Deposited {} into account {}", amount, account_id.0);
        Ok(())
    }
}

#[async_trait]
impl AccountPort for SqliteAccountStore {
    async fn freeze_funds(&self, account_id: &AccountId, amount: Decimal) -> Result<(), TradeError> {
        let pool = self.get_or_init_pool(&account_id.0).await?;
        let mut tx = pool.begin().await.map_err(|e| TradeError::InternalError(e.to_string()))?;

        let row: (String, String) = sqlx::query_as("SELECT available_balance, frozen_balance FROM asset_status WHERE id = 'MAIN'")
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| TradeError::InternalError(e.to_string()))?;

        let mut avail = Decimal::from_str(&row.0).unwrap_or_default();
        let mut frozen = Decimal::from_str(&row.1).unwrap_or_default();

        if avail < amount {
            return Err(TradeError::InsufficientFunds {
                required: amount,
                actual: avail,
            });
        }

        avail -= amount;
        frozen += amount;

        sqlx::query("UPDATE asset_status SET available_balance = ?, frozen_balance = ?, updated_at = ? WHERE id = 'MAIN'")
            .bind(avail.to_string())
            .bind(frozen.to_string())
            .bind(Utc::now())
            .execute(&mut *tx)
            .await
            .map_err(|e| TradeError::InternalError(e.to_string()))?;
            
        sqlx::query("INSERT INTO trade_ledger (action_type, asset_change, frozen_change, created_at) VALUES (?, ?, ?, ?)")
            .bind("FreezeFunds")
            .bind((-amount).to_string())
            .bind(amount.to_string())
            .bind(Utc::now())
            .execute(&mut *tx)
            .await
            .map_err(|e| TradeError::InternalError(e.to_string()))?;

        tx.commit().await.map_err(|e| TradeError::InternalError(e.to_string()))?;
        Ok(())
    }

    async fn unfreeze_funds(&self, account_id: &AccountId, amount: Decimal) -> Result<(), TradeError> {
        let pool = self.get_or_init_pool(&account_id.0).await?;
        let mut tx = pool.begin().await.map_err(|e| TradeError::InternalError(e.to_string()))?;

        let row: (String, String) = sqlx::query_as("SELECT available_balance, frozen_balance FROM asset_status WHERE id = 'MAIN'")
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| TradeError::InternalError(e.to_string()))?;

        let mut avail = Decimal::from_str(&row.0).unwrap_or_default();
        let mut frozen = Decimal::from_str(&row.1).unwrap_or_default();

        let actual_unfreeze = if amount > frozen {
            warn!("账户 {} 解冻异常: 试图解冻 {} 但仅剩 {}", account_id.0, amount, frozen);
            frozen
        } else {
            amount
        };

        frozen -= actual_unfreeze;
        avail += actual_unfreeze;

        sqlx::query("UPDATE asset_status SET available_balance = ?, frozen_balance = ?, updated_at = ? WHERE id = 'MAIN'")
            .bind(avail.to_string())
            .bind(frozen.to_string())
            .bind(Utc::now())
            .execute(&mut *tx)
            .await
            .map_err(|e| TradeError::InternalError(e.to_string()))?;

        sqlx::query("INSERT INTO trade_ledger (action_type, asset_change, frozen_change, created_at) VALUES (?, ?, ?, ?)")
            .bind("UnfreezeFunds")
            .bind(actual_unfreeze.to_string())
            .bind((-actual_unfreeze).to_string())
            .bind(Utc::now())
            .execute(&mut *tx)
            .await
            .map_err(|e| TradeError::InternalError(e.to_string()))?;

        tx.commit().await.map_err(|e| TradeError::InternalError(e.to_string()))?;
        Ok(())
    }

    async fn process_trade(&self, account_id: &AccountId, trade: &Trade, est_req_funds: Decimal) -> Result<(), TradeError> {
        let pool = self.get_or_init_pool(&account_id.0).await?;
        let mut tx = pool.begin().await.map_err(|e| TradeError::InternalError(e.to_string()))?;

        // 1. 获取账金
        let row: (String, String) = sqlx::query_as("SELECT available_balance, frozen_balance FROM asset_status WHERE id = 'MAIN'")
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| TradeError::InternalError(e.to_string()))?;
        let mut avail = Decimal::from_str(&row.0).unwrap_or_default();
        let mut frozen = Decimal::from_str(&row.1).unwrap_or_default();

        let mut ledger_asset_change = Decimal::ZERO;
        let mut ledger_frozen_change = Decimal::ZERO;

        // 2. 资金结转
        if trade.direction == OrderDirection::Buy {
            let actual_cost = trade.price * trade.volume + trade.commission;
            
            if frozen >= actual_cost {
                frozen -= actual_cost;
                ledger_frozen_change -= actual_cost;
            } else {
                let remain = actual_cost - frozen;
                ledger_frozen_change -= frozen;
                frozen = Decimal::ZERO;
                avail -= remain;
                ledger_asset_change -= remain;
            }

            let over_frozen = est_req_funds - actual_cost;
            if over_frozen > Decimal::ZERO {
                let actual_unfreeze = if over_frozen > frozen { frozen } else { over_frozen };
                frozen -= actual_unfreeze;
                avail += actual_unfreeze;
                ledger_frozen_change -= actual_unfreeze;
                ledger_asset_change += actual_unfreeze;
            }
        } else {
            let actual_gain = trade.price * trade.volume - trade.commission;
            avail += actual_gain;
            ledger_asset_change += actual_gain;
        }

        sqlx::query("UPDATE asset_status SET available_balance = ?, frozen_balance = ?, updated_at = ? WHERE id = 'MAIN'")
            .bind(avail.to_string())
            .bind(frozen.to_string())
            .bind(Utc::now())
            .execute(&mut *tx)
            .await
            .map_err(|e| TradeError::InternalError(e.to_string()))?;

        // 3. 持仓清算
        let delta_volume = if trade.direction == OrderDirection::Buy { trade.volume } else { -trade.volume };
        let mut pos_vol = Decimal::ZERO;
        let mut pos_price = Decimal::ZERO;
        
        let existing_pos: Option<(String, String)> = sqlx::query_as("SELECT quantity, avg_price FROM positions WHERE symbol = ?")
            .bind(&trade.symbol)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| TradeError::InternalError(e.to_string()))?;

        if let Some((qv, qp)) = existing_pos {
            pos_vol = Decimal::from_str(&qv).unwrap_or_default();
            pos_price = Decimal::from_str(&qp).unwrap_or_default();
        }

        if (pos_vol.is_sign_positive() && delta_volume.is_sign_positive())
            || (pos_vol.is_sign_negative() && delta_volume.is_sign_negative())
            || pos_vol.is_zero()
        {
            let old_cost = pos_vol.abs() * pos_price;
            let added_cost = delta_volume.abs() * trade.price;
            pos_vol += delta_volume;
            if !pos_vol.is_zero() {
                pos_price = (old_cost + added_cost) / pos_vol.abs();
            }
        } else {
            pos_vol += delta_volume;
            if pos_vol.is_zero() {
                pos_price = Decimal::ZERO;
            } else if (pos_vol.is_sign_positive() && delta_volume.is_sign_negative())
                || (pos_vol.is_sign_negative() && delta_volume.is_sign_positive())
            {
                pos_price = trade.price;
            }
        }

        sqlx::query("INSERT OR REPLACE INTO positions (symbol, quantity, avg_price, updated_at) VALUES (?, ?, ?, ?)")
            .bind(&trade.symbol)
            .bind(pos_vol.to_string())
            .bind(pos_price.to_string())
            .bind(Utc::now())
            .execute(&mut *tx)
            .await
            .map_err(|e| TradeError::InternalError(e.to_string()))?;

        // 4. Ledger 明细落地
        sqlx::query("INSERT INTO trade_ledger (action_type, asset_change, frozen_change, created_at) VALUES (?, ?, ?, ?)")
            .bind("TradeFilled")
            .bind(ledger_asset_change.to_string())
            .bind(ledger_frozen_change.to_string())
            .bind(Utc::now())
            .execute(&mut *tx)
            .await
            .map_err(|e| TradeError::InternalError(e.to_string()))?;

        tx.commit().await.map_err(|e| TradeError::InternalError(e.to_string()))?;
        Ok(())
    }

    async fn snapshot(&self, account_id: &AccountId) -> Result<AccountSnapshot, TradeError> {
        let pool = self.get_or_init_pool(&account_id.0).await?;
        
        let row: (String, String) = sqlx::query_as("SELECT available_balance, frozen_balance FROM asset_status WHERE id = 'MAIN'")
            .fetch_one(&pool)
            .await
            .map_err(|e| TradeError::InternalError(e.to_string()))?;

        let available_balance = Decimal::from_str(&row.0).unwrap_or_default();
        let frozen_balance = Decimal::from_str(&row.1).unwrap_or_default();
        let total_equity = available_balance + frozen_balance;

        let cur_positions = sqlx::query_as::<_, (String, String, String)>("SELECT symbol, quantity, avg_price FROM positions")
            .fetch_all(&pool)
            .await
            .map_err(|e| TradeError::InternalError(e.to_string()))?;

        let mut positions = Vec::new();
        for p in cur_positions {
            let vol = Decimal::from_str(&p.1).unwrap_or_default();
            if !vol.is_zero() {
                positions.push(Position {
                    account_id: account_id.clone(),
                    symbol: p.0,
                    volume: vol,
                    average_price: Decimal::from_str(&p.2).unwrap_or_default(),
                });
            }
        }

        Ok(AccountSnapshot {
            account_id: account_id.clone(),
            available_balance,
            frozen_balance,
            total_equity,
            positions,
        })
    }
}
