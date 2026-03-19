use chrono::{TimeZone, Utc};
use okane_core::common::{Stock, TimeFrame};
use okane_core::market::entity::Candle;
use okane_core::store::port::{MarketStore, Position, StockMetadata, SystemStore, User};
use okane_core::trade::entity::{AccountId, Order, OrderDirection, OrderId, OrderStatus};
use okane_core::trade::port::PendingOrderPort;
use okane_store::config::set_root_dir;
use okane_store::market::SqliteMarketStore;
use okane_store::pending_order_sqlx::SqlitePendingOrderStore;
use okane_store::system::SqliteSystemStore;
use rust_decimal_macros::dec;
use tempfile::tempdir;

#[tokio::test]
async fn test_store_full_integration() -> anyhow::Result<()> {
    // 1. 初始化临时测试环境
    let tmp_dir = tempdir().map_err(|e| anyhow::anyhow!("Failed to create temp dir: {}", e))?;
    let root_path = tmp_dir.path().to_path_buf();
    // 2. 测试 SqliteSystemStore
    let system_store = SqliteSystemStore::new_with_path(Some(root_path.clone()))
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create system store: {}", e))?;

    // 用户存取
    let user = User {
        id: "u1".to_string(),
        name: "Tester".to_string(),
        password_hash: "dummy_hash".to_string(),
        role: okane_core::store::port::UserRole::Standard,
        force_password_change: false,
        created_at: Utc::now(),
    };
    system_store
        .save_user(&user)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    let saved_user = system_store
        .get_user("u1")
        .await
        .map_err(|e| anyhow::anyhow!(e))?
        .ok_or_else(|| anyhow::anyhow!("User should exist"))?;
    assert_eq!(saved_user.name, "Tester");

    // 自选股
    system_store
        .add_to_watchlist("u1", "AAPL")
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    let watchlist = system_store
        .get_watchlist("u1")
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    assert!(watchlist.contains(&"AAPL".to_string()));

    // 持仓
    let pos = Position {
        symbol: "AAPL".to_string(),
        quantity: dec!(100.0),
        avg_price: dec!(150.0),
        last_updated: Utc::now(),
    };
    system_store
        .update_position("u1", &pos)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    let positions = system_store
        .get_positions("u1")
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    assert_eq!(positions[0].quantity, dec!(100.0));

    // 股票搜索
    let meta = StockMetadata {
        symbol: "TSLA".to_string(),
        name: "Tesla Inc".to_string(),
        exchange: "NASDAQ".to_string(),
        sector: Some("Auto".to_string()),
        currency: "USD".to_string(),
    };
    system_store
        .save_stock_metadata(&meta)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    let search_results = system_store
        .search_stocks("Tesla")
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    assert_eq!(search_results.len(), 1);

    // 3. 测试 SqliteMarketStore
    let market_store = SqliteMarketStore::new_with_path(Some(root_path.clone()))
        .map_err(|e| anyhow::anyhow!("Failed to create market store: {}", e))?;
    let stock = Stock {
        symbol: "AAPL".into(),
        exchange: Some("NASDAQ".into()),
    };
    let candles = vec![Candle {
        time: Utc
            .with_ymd_and_hms(2026, 2, 1, 10, 0, 0)
            .single()
            .ok_or_else(|| anyhow::anyhow!("Invalid date"))?,
        open: dec!(150.0),
        high: dec!(155.0),
        low: dec!(149.0),
        close: dec!(152.0),
        adj_close: Some(dec!(152.0)),
        volume: dec!(10000.0),
        is_final: true,
    }];

    market_store
        .save_candles(&stock, TimeFrame::Day1, &candles)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    // 验证物理路径 (应当在临时目录下)
    let db_file = root_path.join("market").join("AAPL_NASDAQ.db");
    assert!(db_file.exists());

    // 读取验证
    let start = Utc
        .with_ymd_and_hms(2026, 2, 1, 0, 0, 0)
        .single()
        .ok_or_else(|| anyhow::anyhow!("Invalid date"))?;
    let end = Utc
        .with_ymd_and_hms(2026, 2, 1, 23, 59, 59)
        .single()
        .ok_or_else(|| anyhow::anyhow!("Invalid date"))?;
    let loaded = market_store
        .load_candles(&stock, TimeFrame::Day1, start, end)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].close, dec!(152.0));
    assert_eq!(loaded[0].adj_close, Some(dec!(152.0)));
    assert!(loaded[0].is_final);
    Ok(())
}

#[tokio::test]
async fn test_pending_order_store_recovers_orders_after_restart() -> anyhow::Result<()> {
    let tmp_dir = tempdir().map_err(|e| anyhow::anyhow!("Failed to create temp dir: {}", e))?;
    let root_path = tmp_dir.path().to_path_buf();
    set_root_dir(root_path.clone());

    let store = SqlitePendingOrderStore::new_with_path(Some(root_path.clone()))?;
    let order_id = OrderId("restart-order-1".to_string());
    let order = Order {
        id: order_id.clone(),
        account_id: AccountId("acct_restart".to_string()),
        symbol: "AAPL".to_string(),
        direction: OrderDirection::Buy,
        price: Some(dec!(150.0)),
        volume: dec!(10.0),
        filled_volume: dec!(0.0),
        status: OrderStatus::Pending,
        created_at: Utc::now().timestamp_millis(),
    };
    store.save(order.clone()).await?;
    drop(store);

    let restarted = SqlitePendingOrderStore::new_with_path(Some(root_path))?;
    let recovered = restarted
        .get(&order_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("order should be recoverable after restart"))?;
    assert_eq!(recovered.account_id.0, "acct_restart");

    let by_symbol = restarted.get_by_symbol("AAPL").await?;
    assert_eq!(by_symbol.len(), 1);

    let removed = restarted
        .remove(&order_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("order should be removable after restart"))?;
    assert_eq!(removed.id.0, order.id.0);

    Ok(())
}
